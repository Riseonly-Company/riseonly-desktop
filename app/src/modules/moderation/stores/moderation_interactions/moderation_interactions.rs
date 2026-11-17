use std::collections::HashMap;
use std::future::Future;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, AsyncApp, Context, Entity, Task, WeakEntity};
use rise_canon::RiseEvent;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_events::name;
use rise_i18n::tr;
use rise_moderation::BlockTypeRule;
use rise_platform::haptics::{NotificationKind, notification};
use serde_json::Value;

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::lib::sai::modules::sai_ws::rise_event_payload::RiseEventPayload;
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWs;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pagination::SaiWsPaginationEdge;
use crate::core::lib::sai::sai::Sai;
use crate::core::rise_engine_bridge::rise_transport_status::{RiseTransportDropWatch, RiseTransportStatus};
use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierIcon, NotifierPosition, NotifierRetryOptions, NotifierSendOptions, NotifierType,
};
use crate::modules::diagnostics::engine::core::rise_bug_report_upload::BugReportAttachment;
use crate::modules::diagnostics::stores::diagnostics::diagnostics_interactions::bug_report_interactions::BugReportAttachmentFactory;
use crate::modules::moderation::engine::rise_moderation_domain::{RiseModerationDomain, RiseModerationDomainRegistry};
use crate::modules::moderation::engine::rise_moderation_failure::RiseModerationFailure;
use crate::modules::moderation::engine::rise_moderation_presentation::{
    ModerationLoadState, RiseModerationPresentation,
};
use crate::modules::moderation::engine::rise_moderation_repository::RiseModerationRepository;
use crate::modules::moderation::engine::rise_moderation_submitter::{
    AppealDraftPayload, NoteDraftPayload, ReportDraftPayload, RiseModerationSubmissionOwner, RiseModerationSubmitter,
};
use crate::modules::moderation::engine::rise_moderation_wire::{ReportResolutionEvent, RiseStoredUserBlock};
use crate::modules::moderation::stores::moderation_actions::moderation_actions::{
    ModerationActionDone, ModerationActionsStore,
};
use crate::modules::moderation::stores::moderation_actions::moderation_types::{
    AdminSanctionKind, AppealDecision, AppealDecisionExt, AppealReviewFilter, AppealStatus, AppealStatusFilter,
    ModerationMuteScope, ModeratorKindFilter, ReportQueueSort, ReportReason, ReportReasonExt, ReportStatus,
    ReportTarget, ReportTargetType, ResolveAction, ResolveActionExt, UserBlockType,
};
use crate::modules::moderation::stores::moderation_actions::moderation_wire_types::{
    ModeratorStatsWireDTO, ModeratorWithUserWireDTO,
};
use crate::modules::moderation::stores::moderation_services::moderation_services::ModerationServicesStore;

const MY_REPORTS_SURFACE: &str = "myReports";
const BLOCKS_SURFACE: &str = "blocks";
const BLOCKED_ACCOUNT_APPEAL_SURFACE: &str = "blockedAccountAppeal";
const MY_APPEALS_SURFACE: &str = "myAppeals";
const QUEUE_SURFACE: &str = "queue";
const REVIEW_SURFACE: &str = "review";

pub struct ModerationInteractionsStore {
    blocked_account_appeal_revision: u64,
    generations: HashMap<String, u64>,
    open_case_screens: HashMap<i64, i64>,
    my_reports_drop_watch: Option<RiseTransportDropWatch>,
    opens_on_appeals: bool,
    blocks_drop_watch: Option<RiseTransportDropWatch>,
    blocks_landed_generation: u64,
}

impl ModerationInteractionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            blocked_account_appeal_revision: 0,
            generations: HashMap::new(),
            open_case_screens: HashMap::new(),
            my_reports_drop_watch: None,
            opens_on_appeals: false,
            blocks_drop_watch: None,
            blocks_landed_generation: 0,
        }
    }

    pub fn blocked_account_appeal_revision(&self) -> u64 {
        self.blocked_account_appeal_revision
    }

    fn bump_blocked_account_appeal_revision(&mut self, cx: &mut Context<Self>) {
        self.blocked_account_appeal_revision = self.blocked_account_appeal_revision.wrapping_add(1);
        cx.notify();
    }

    fn next_generation(&mut self, surface: &str) -> u64 {
        let generation = self.generations.entry(surface.to_owned()).or_insert(0);
        *generation = generation.wrapping_add(1);
        *generation
    }

    fn is_current(&self, surface: &str, generation: u64) -> bool {
        self.generations.get(surface) == Some(&generation)
    }

    fn current_generation(&self, surface: &str) -> u64 {
        self.generations.get(surface).copied().unwrap_or(0)
    }

    fn still_current(this: &WeakEntity<Self>, surface: &str, generation: u64, cx: &mut AsyncApp) -> bool {
        this.read_with(cx, |this, _| this.is_current(surface, generation)).unwrap_or(false)
    }

    fn domain(cx: &mut App) -> Task<anyhow::Result<Arc<RiseModerationDomain>>> {
        RiseModerationDomainRegistry::shared(cx).update(cx, |registry, cx| registry.require_domain(cx))
    }

    fn is_active_domain(domain: &Arc<RiseModerationDomain>, cx: &mut App) -> bool {
        RiseModerationDomainRegistry::shared(cx)
            .read(cx)
            .active()
            .is_some_and(|active| Arc::ptr_eq(active, domain))
    }

    async fn on_account_runtime<T: Send + 'static>(
        operation: impl Future<Output = T> + Send + 'static,
    ) -> anyhow::Result<T> {
        RiseAccountRuntime::current().spawn(operation).await.map_err(anyhow::Error::new)
    }

    fn read_presentation<R>(cx: &mut App, read: impl FnOnce(&RiseModerationPresentation) -> R) -> R {
        let presentation = RiseModerationPresentation::shared(cx);
        read(presentation.read(cx))
    }

    fn update_presentation<R>(
        cx: &mut App,
        update: impl FnOnce(&mut RiseModerationPresentation, &mut Context<RiseModerationPresentation>) -> R,
    ) -> R {
        RiseModerationPresentation::shared(cx).update(cx, update)
    }

    fn can_request_continuation(cache_key: &str, cx: &mut App) -> bool {
        SaiWs::pagination_state(cache_key, cx)
            .read(cx)
            .state(SaiWsPaginationEdge::Trailing)
            .can_request_automatically()
    }

    fn send_toast(title: String, content: String, kind: NotifierType, icon: &str, cx: &mut App) {
        Notifier::send(
            title,
            NotifierSendOptions {
                content: Some(content),
                r#type: kind,
                position: NotifierPosition::Bottom,
                icon: NotifierIcon::System(icon.to_owned()),
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    fn is_cancellation(error: &anyhow::Error) -> bool {
        error.downcast_ref::<CancellationError>().is_some()
    }

    pub fn submit_report(&mut self, draft: ReportDraftPayload, cx: &mut Context<Self>) {
        let owner = RiseModerationSubmissionOwner::new(cx);
        cx.spawn(async move |this, cx| {
            let mut owner = owner;
            let outcome = match owner.clone().prepared(cx).await {
                Ok(prepared) => {
                    owner = prepared;
                    RiseModerationSubmitter::shared().submit_report(draft.clone(), owner.clone(), cx).await
                }
                Err(error) => Err(error),
            };
            match outcome {
                Ok(report) => {
                    if !owner.is_same_account() {
                        return;
                    }
                    let refreshes = report.reported_user.is_none();
                    cx.update(|cx| {
                        Self::update_presentation(cx, |presentation, cx| presentation.prepend_my_report(report, cx));
                        let loaded =
                            Self::read_presentation(cx, |presentation| *presentation.my_reports_state() == ModerationLoadState::Loaded);
                        if loaded && refreshes {
                            let _ = this.update(cx, |this, cx| this.refresh_my_reports(cx));
                        }
                        notification(NotificationKind::Success);
                        Self::send_toast(
                            tr("report_sent_title"),
                            tr("report_sent_content"),
                            NotifierType::Success,
                            "checkmark.circle.fill",
                            cx,
                        );
                    });
                }
                Err(error) => {
                    let diagnostic = RiseModerationFailure::diagnostic(&error);
                    cx.update(|cx| {
                        Sai::log("Report", move || format!("createReport error: {diagnostic}"), SaiLogKind::Network, cx);
                        if Self::is_cancellation(&error) || !owner.is_same_account() {
                            RiseModerationSubmitter::shared().discard_report(&draft);
                            return;
                        }
                        if RiseModerationFailure::is_duplicate(&error) {
                            RiseModerationSubmitter::shared().discard_report(&draft);
                            Self::send_toast(
                                tr("report_duplicate_title"),
                                tr("report_duplicate_content"),
                                NotifierType::Warning,
                                "exclamationmark.triangle.fill",
                                cx,
                            );
                            return;
                        }
                        let expired_draft = draft.clone();
                        let retried_draft = draft;
                        Notifier::send_retry(
                            tr("report_failed_title"),
                            NotifierRetryOptions {
                                content: Some(RiseModerationFailure::message(&error)),
                                ..NotifierRetryOptions::default()
                            },
                            move |_cx| RiseModerationSubmitter::shared().discard_report(&expired_draft),
                            move |cx| {
                                Self::shared(cx).update(cx, |this, cx| this.submit_report(retried_draft, cx));
                            },
                            cx,
                        );
                    });
                }
            }
        })
        .detach();
    }

    pub fn can_send_report_now(&mut self, cx: &mut Context<Self>) -> Task<bool> {
        let Some(domain) = RiseModerationDomainRegistry::shared(cx).read(cx).active().cloned() else {
            return Task::ready(true);
        };
        cx.spawn(async move |_, cx| {
            let engine = domain.engine.clone();
            let ready = Self::on_account_runtime(async move { engine.is_ready_for_calls(false).await }).await;
            if !matches!(ready, Ok(Ok(false))) {
                return true;
            }
            cx.update(|cx| {
                notification(NotificationKind::Error);
                Self::send_toast(
                    tr("report_failed_title"),
                    tr("moderation_error_connection"),
                    NotifierType::Warning,
                    "exclamationmark.triangle.fill",
                    cx,
                );
            });
            false
        })
    }

    pub fn report_chat_spam(&mut self, chat_id: &str, message_id: Option<&str>, cx: &mut Context<Self>) {
        if chat_id.is_empty() {
            return;
        }
        let target = match message_id {
            Some(message_id) => ReportTarget::Message { chat_id: chat_id.to_owned(), message_id: message_id.to_owned() },
            None => ReportTarget::Chat(chat_id.to_owned()),
        };
        self.submit_report(
            ReportDraftPayload {
                draft_id: Uuid::new_v4().uuid_string(),
                target,
                reason: ReportReason::Spam,
                description: tr(ReportReason::Spam.title_key()),
                attachments: Vec::new(),
                include_context: message_id.is_some(),
            },
            cx,
        );
    }

    pub fn ensure_my_reports_loaded(&mut self, cx: &mut Context<Self>) {
        if Self::read_presentation(cx, |presentation| presentation.is_stale()) {
            Self::update_presentation(cx, |presentation, cx| presentation.clear_stale(cx));
            self.load_my_reports(true, cx);
            return;
        }
        if Self::read_presentation(cx, |presentation| *presentation.my_reports_state() == ModerationLoadState::Loading) {
            return;
        }
        self.load_my_reports(false, cx);
    }

    pub fn refresh_my_reports(&mut self, cx: &mut Context<Self>) {
        self.load_my_reports(true, cx);
    }

    pub fn set_my_reports_filter(&mut self, status: Option<ReportStatus>, cx: &mut Context<Self>) {
        if Self::read_presentation(cx, |presentation| presentation.my_reports_filter() == status) {
            return;
        }
        Self::update_presentation(cx, |presentation, cx| presentation.set_my_reports_filter(status, cx));
        self.load_my_reports(false, cx);
    }

    fn load_my_reports(&mut self, force: bool, cx: &mut Context<Self>) {
        let filter = Self::read_presentation(cx, |presentation| presentation.my_reports_filter());
        let generation = self.next_generation(MY_REPORTS_SURFACE);
        if Self::read_presentation(cx, |presentation| presentation.my_reports().is_empty()) {
            Self::update_presentation(cx, |presentation, cx| {
                presentation.set_my_reports_state(ModerationLoadState::Loading, cx)
            });
        }
        if force {
            RiseTransportStatus::wake_if_offline(cx);
        }
        let offline = RiseTransportStatus::is_offline(cx);
        if RiseTransportStatus::is_connecting(cx) {
            self.fail_my_reports_on_socket_drop(generation, cx);
        }
        let domain = Self::domain(cx);
        cx.spawn(async move |this, cx| {
            Self::load_my_reports_body(&this, domain, filter, force, offline, generation, cx).await;
            let _ = this.update(cx, |this, _| {
                if this.is_current(MY_REPORTS_SURFACE, generation) {
                    this.stop_my_reports_drop_watch();
                }
            });
        })
        .detach();
    }

    async fn load_my_reports_body(
        this: &WeakEntity<Self>,
        domain: Task<anyhow::Result<Arc<RiseModerationDomain>>>,
        filter: Option<ReportStatus>,
        force: bool,
        offline: bool,
        generation: u64,
        cx: &mut AsyncApp,
    ) {
        let result: anyhow::Result<()> = async {
            let domain = domain.await?;
            let repository: Arc<RiseModerationRepository> = domain.repository.clone();
            let empty = cx.update(|cx| Self::read_presentation(cx, |presentation| presentation.my_reports().is_empty()));
            if !force && empty {
                let cached_repository = repository.clone();
                let cached = Self::on_account_runtime(async move { cached_repository.cached_my_reports(filter).await })
                    .await
                    .ok()
                    .flatten();
                if let Some(cached) = cached {
                    if !Self::still_current(this, MY_REPORTS_SURFACE, generation, cx) {
                        return anyhow::Ok(());
                    }
                    cx.update(|cx| {
                        Self::update_presentation(cx, |presentation, cx| presentation.apply_my_reports(cached.value, false, cx))
                    });
                }
            }
            let still_empty = cx.update(|cx| Self::read_presentation(cx, |presentation| presentation.my_reports().is_empty()));
            if offline && still_empty && Self::still_current(this, MY_REPORTS_SURFACE, generation, cx) {
                cx.update(|cx| {
                    Self::update_presentation(cx, |presentation, cx| {
                        presentation.fail_my_reports(tr("moderation_error_connection"), cx)
                    })
                });
            }
            let page = Self::on_account_runtime(async move { repository.my_reports(filter, None).await }).await??;
            if !Self::still_current(this, MY_REPORTS_SURFACE, generation, cx) {
                return anyhow::Ok(());
            }
            cx.update(|cx| Self::update_presentation(cx, |presentation, cx| presentation.apply_my_reports(page, false, cx)));
            anyhow::Ok(())
        }
        .await;
        if let Err(error) = result {
            if !Self::still_current(this, MY_REPORTS_SURFACE, generation, cx) {
                return;
            }
            let message = RiseModerationFailure::message(&error);
            let diagnostic = RiseModerationFailure::diagnostic(&error);
            cx.update(|cx| {
                Self::update_presentation(cx, |presentation, cx| presentation.fail_my_reports(message, cx));
                Sai::log("Report", move || format!("getMyReports error: {diagnostic}"), SaiLogKind::Network, cx);
            });
        }
    }

    fn fail_my_reports_on_socket_drop(&mut self, generation: u64, cx: &mut Context<Self>) {
        self.stop_my_reports_drop_watch();
        let this = cx.weak_entity();
        self.my_reports_drop_watch = Some(RiseTransportStatus::on_next_disconnect(
            move |cx| {
                let _ = this.update(cx, |this, cx| {
                    if !this.is_current(MY_REPORTS_SURFACE, generation) {
                        return;
                    }
                    this.my_reports_drop_watch = None;
                    let waiting = Self::read_presentation(cx, |presentation| {
                        presentation.my_reports().is_empty()
                            && *presentation.my_reports_state() == ModerationLoadState::Loading
                    });
                    if !waiting {
                        return;
                    }
                    Self::update_presentation(cx, |presentation, cx| {
                        presentation.fail_my_reports(tr("moderation_error_connection"), cx)
                    });
                });
            },
            cx,
        ));
    }

    fn stop_my_reports_drop_watch(&mut self) {
        if let Some(watch) = &self.my_reports_drop_watch {
            watch.stop();
        }
        self.my_reports_drop_watch = None;
    }

    pub fn load_more_my_reports(&mut self, cx: &mut Context<Self>) {
        let (has_more, is_loading_more, cursor, filter) = Self::read_presentation(cx, |presentation| {
            (
                presentation.my_reports_has_more(),
                presentation.is_loading_more_my_reports(),
                presentation.my_reports_cursor(),
                presentation.my_reports_filter(),
            )
        });
        if !has_more || is_loading_more {
            return;
        }
        let Some(cursor) = cursor else {
            return;
        };
        let generation = self.current_generation(MY_REPORTS_SURFACE);
        if RiseTransportStatus::is_offline(cx) {
            RiseTransportStatus::wake_if_offline(cx);
            Self::update_presentation(cx, |presentation, cx| {
                presentation.set_loading_more_my_reports(true, cx);
                presentation.fail_more_my_reports(tr("moderation_error_connection"), cx);
            });
            return;
        }
        Self::update_presentation(cx, |presentation, cx| presentation.set_loading_more_my_reports(true, cx));
        let drop_watch = if RiseTransportStatus::is_connecting(cx) {
            let this = cx.weak_entity();
            Some(RiseTransportStatus::on_next_disconnect(
                move |cx| {
                    let current = this
                        .read_with(cx, |this, _| this.is_current(MY_REPORTS_SURFACE, generation))
                        .unwrap_or(false);
                    if !current {
                        return;
                    }
                    Self::update_presentation(cx, |presentation, cx| {
                        presentation.fail_more_my_reports(tr("moderation_error_connection"), cx)
                    });
                },
                cx,
            ))
        } else {
            None
        };
        let domain = Self::domain(cx);
        cx.spawn(async move |this, cx| {
            let result: anyhow::Result<_> = async {
                let domain = domain.await?;
                let repository = domain.repository.clone();
                Self::on_account_runtime(async move { repository.my_reports(filter, Some(cursor)).await }).await?
            }
            .await;
            if let Some(watch) = &drop_watch {
                watch.stop();
            }
            if !Self::still_current(&this, MY_REPORTS_SURFACE, generation, cx) {
                return;
            }
            cx.update(|cx| match result {
                Ok(page) => Self::update_presentation(cx, |presentation, cx| presentation.apply_my_reports(page, true, cx)),
                Err(error) => Self::update_presentation(cx, |presentation, cx| {
                    presentation.fail_more_my_reports(RiseModerationFailure::message(&error), cx)
                }),
            });
        })
        .detach();
    }

    pub fn open_my_blocks_on_appeals(&mut self) {
        self.opens_on_appeals = true;
    }

    pub fn take_appeals_tab_request(&mut self) -> bool {
        let request = self.opens_on_appeals;
        self.opens_on_appeals = false;
        request
    }

    pub fn ensure_blocks_loaded(&mut self, cx: &mut Context<Self>) {
        if Self::read_presentation(cx, |presentation| presentation.is_stale()) {
            Self::update_presentation(cx, |presentation, cx| presentation.clear_stale(cx));
            self.load_blocks(cx);
            return;
        }
        if Self::read_presentation(cx, |presentation| *presentation.my_blocks_state() == ModerationLoadState::Loading) {
            return;
        }
        self.load_blocks(cx);
    }

    pub fn refresh_blocks(&mut self, cx: &mut Context<Self>) {
        self.load_blocks(cx);
    }

    pub fn refresh_blocks_awaiting_reply(&mut self, cx: &mut Context<Self>) -> Task<Option<Vec<RiseStoredUserBlock>>> {
        let generation = self.next_generation(BLOCKS_SURFACE);
        if Self::read_presentation(cx, |presentation| presentation.my_blocks().is_empty()) {
            Self::update_presentation(cx, |presentation, cx| presentation.set_blocks_state(ModerationLoadState::Loading, cx));
        }
        RiseTransportStatus::wake_if_offline(cx);
        if RiseTransportStatus::is_offline(cx) {
            self.fail_blocks_while_offline(generation, cx);
        } else if RiseTransportStatus::is_connecting(cx) {
            if let Some(watch) = &self.blocks_drop_watch {
                watch.stop();
            }
            let this = cx.weak_entity();
            self.blocks_drop_watch = Some(RiseTransportStatus::on_next_disconnect(
                move |cx| {
                    let _ = this.update(cx, |this, cx| {
                        if !this.is_current(BLOCKS_SURFACE, generation) {
                            return;
                        }
                        this.blocks_drop_watch = None;
                        this.fail_blocks_while_offline(generation, cx);
                    });
                },
                cx,
            ));
        }
        let domain = Self::domain(cx);
        cx.spawn(async move |this, cx| {
            let blocks = Self::refresh_blocks_awaiting_reply_body(&this, domain, generation, cx).await;
            let _ = this.update(cx, |this, _| {
                if this.is_current(BLOCKS_SURFACE, generation) {
                    if let Some(watch) = &this.blocks_drop_watch {
                        watch.stop();
                    }
                    this.blocks_drop_watch = None;
                }
            });
            blocks
        })
    }

    async fn refresh_blocks_awaiting_reply_body(
        this: &WeakEntity<Self>,
        domain: Task<anyhow::Result<Arc<RiseModerationDomain>>>,
        generation: u64,
        cx: &mut AsyncApp,
    ) -> Option<Vec<RiseStoredUserBlock>> {
        let result: anyhow::Result<_> = async {
            let domain = domain.await?;
            let repository = domain.repository.clone();
            let blocks = Self::on_account_runtime(async move { repository.my_blocks().await }).await??;
            anyhow::Ok((domain, blocks))
        }
        .await;
        match result {
            Ok((domain, blocks)) => {
                let lands = this.update(cx, |this, cx| this.lands_blocks(generation, &domain, cx)).unwrap_or(false);
                if !lands {
                    return None;
                }
                let landed = blocks.blocks.clone();
                cx.update(|cx| Self::update_presentation(cx, |presentation, cx| presentation.apply_blocks(blocks, cx)));
                Some(landed)
            }
            Err(error) => {
                if !Self::still_current(this, BLOCKS_SURFACE, generation, cx) {
                    return None;
                }
                let message = RiseModerationFailure::message(&error);
                let diagnostic = RiseModerationFailure::diagnostic(&error);
                cx.update(|cx| {
                    Self::update_presentation(cx, |presentation, cx| presentation.fail_blocks(message, cx));
                    Sai::log("Blocks", move || format!("getMyBlocks error: {diagnostic}"), SaiLogKind::Network, cx);
                });
                None
            }
        }
    }

    fn lands_blocks(&mut self, generation: u64, domain: &Arc<RiseModerationDomain>, cx: &mut Context<Self>) -> bool {
        if generation <= self.blocks_landed_generation {
            return false;
        }
        if !self.is_current(BLOCKS_SURFACE, generation) {
            let same_account =
                SaiAccountCacheScope::is_current_account_epoch(Some(&domain.account_id), Some(&domain.account_epoch));
            if !same_account || !Self::is_active_domain(domain, cx) {
                return false;
            }
        }
        self.blocks_landed_generation = generation;
        true
    }

    fn fail_blocks_while_offline(&mut self, generation: u64, cx: &mut Context<Self>) {
        if !self.is_current(BLOCKS_SURFACE, generation) {
            return;
        }
        let waiting = Self::read_presentation(cx, |presentation| {
            presentation.my_blocks().is_empty() && *presentation.my_blocks_state() == ModerationLoadState::Loading
        });
        if !waiting {
            return;
        }
        Self::update_presentation(cx, |presentation, cx| presentation.fail_blocks(tr("moderation_error_connection"), cx));
    }

    fn load_blocks(&mut self, cx: &mut Context<Self>) {
        let generation = self.next_generation(BLOCKS_SURFACE);
        if Self::read_presentation(cx, |presentation| presentation.my_blocks().is_empty()) {
            Self::update_presentation(cx, |presentation, cx| presentation.set_blocks_state(ModerationLoadState::Loading, cx));
        }
        let domain = Self::domain(cx);
        cx.spawn(async move |this, cx| {
            let result: anyhow::Result<()> = async {
                let domain = domain.await?;
                let repository = domain.repository.clone();
                let empty = cx.update(|cx| Self::read_presentation(cx, |presentation| presentation.my_blocks().is_empty()));
                if empty {
                    let cached_repository = repository.clone();
                    let cached = Self::on_account_runtime(async move { cached_repository.cached_my_blocks().await })
                        .await
                        .ok()
                        .flatten();
                    if let Some(cached) = cached {
                        if !Self::still_current(&this, BLOCKS_SURFACE, generation, cx) {
                            return anyhow::Ok(());
                        }
                        cx.update(|cx| {
                            Self::update_presentation(cx, |presentation, cx| presentation.apply_blocks(cached.value, cx))
                        });
                    }
                }
                let blocks = Self::on_account_runtime(async move { repository.my_blocks().await }).await??;
                let lands = this.update(cx, |this, cx| this.lands_blocks(generation, &domain, cx)).unwrap_or(false);
                if !lands {
                    return anyhow::Ok(());
                }
                cx.update(|cx| Self::update_presentation(cx, |presentation, cx| presentation.apply_blocks(blocks, cx)));
                anyhow::Ok(())
            }
            .await;
            if let Err(error) = result {
                if !Self::still_current(&this, BLOCKS_SURFACE, generation, cx) {
                    return;
                }
                let message = RiseModerationFailure::message(&error);
                let diagnostic = RiseModerationFailure::diagnostic(&error);
                cx.update(|cx| {
                    Self::update_presentation(cx, |presentation, cx| presentation.fail_blocks(message, cx));
                    Sai::log("Blocks", move || format!("getMyBlocks error: {diagnostic}"), SaiLogKind::Network, cx);
                });
            }
        })
        .detach();
    }

    pub fn submit_appeal(
        &mut self,
        draft: AppealDraftPayload,
        completion: impl FnOnce(bool, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let owner = RiseModerationSubmissionOwner::new(cx);
        cx.spawn(async move |this, cx| {
            let mut owner = owner;
            let outcome = match owner.clone().prepared(cx).await {
                Ok(prepared) => {
                    owner = prepared;
                    let block_id = draft.block_id.clone();
                    cx.update(|cx| {
                        Self::update_presentation(cx, |presentation, cx| presentation.mark_block_appealed(&block_id, true, cx))
                    });
                    RiseModerationSubmitter::shared().submit_appeal(draft.clone(), owner.clone(), cx).await
                }
                Err(error) => Err(error),
            };
            match outcome {
                Ok(appeal) => {
                    if !cx.update(|cx| owner.is_current(cx)) {
                        return;
                    }
                    let _ = this.update(cx, |this, cx| {
                        this.ensure_my_appeals_loaded(cx);
                        if let Some(appeal) = appeal {
                            let block = Self::read_presentation(cx, |presentation| {
                                presentation.my_blocks().iter().find(|block| block.id() == draft.block_id).cloned()
                            });
                            Self::update_presentation(cx, |presentation, cx| {
                                presentation.prepend_my_appeal(appeal, block.as_ref(), cx)
                            });
                        }
                        this.bump_blocked_account_appeal_revision(cx);
                    });
                    cx.update(|cx| {
                        completion(true, cx);
                        notification(NotificationKind::Success);
                        Self::send_toast(
                            tr("appeal_sent_title"),
                            tr("appeal_sent_content"),
                            NotifierType::Success,
                            "checkmark.seal.fill",
                            cx,
                        );
                    });
                }
                Err(error) => {
                    let is_current = cx.update(|cx| owner.is_current(cx));
                    if Self::is_cancellation(&error) || !is_current {
                        let files = draft
                            .attachments
                            .iter()
                            .flat_map(|attachment: &BugReportAttachment| {
                                [attachment.url.clone(), attachment.preview_url.clone()]
                            })
                            .flatten()
                            .collect::<Vec<_>>();
                        BugReportAttachmentFactory::remove_temporary_files(&files);
                        RiseModerationSubmitter::shared().forget(&draft.draft_id);
                        return;
                    }
                    let is_duplicate = RiseModerationFailure::is_duplicate(&error);
                    let is_not_appealable = RiseModerationFailure::is_not_appealable(&error);
                    let diagnostic = RiseModerationFailure::diagnostic(&error);
                    let block_id = draft.block_id.clone();
                    let _ = this.update(cx, |this, cx| {
                        Self::update_presentation(cx, |presentation, cx| {
                            presentation.mark_block_appealed(&block_id, is_duplicate, cx)
                        });
                        if is_duplicate {
                            this.bump_blocked_account_appeal_revision(cx);
                        }
                    });
                    cx.update(|cx| {
                        completion(is_duplicate || is_not_appealable, cx);
                        let (title, content) = if is_duplicate {
                            (tr("appeal_duplicate_title"), tr("appeal_duplicate_content"))
                        } else if is_not_appealable {
                            (tr("appeal_not_appealable_title"), tr("appeal_not_appealable_content"))
                        } else {
                            (tr("appeal_failed_title"), tr("moderation_error_connection"))
                        };
                        Self::send_toast(title, content, NotifierType::Warning, "exclamationmark.triangle.fill", cx);
                        Sai::log("Blocks", move || format!("createAppeal error: {diagnostic}"), SaiLogKind::Network, cx);
                    });
                }
            }
        })
        .detach();
    }

    pub fn blocked_account_appeal_status(
        &mut self,
        block: &RiseStoredUserBlock,
        cx: &mut Context<Self>,
    ) -> Task<Option<AppealStatus>> {
        let user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let generation = self.next_generation(BLOCKED_ACCOUNT_APPEAL_SURFACE);
        let block = block.clone();
        let domain = Self::domain(cx);
        cx.spawn(async move |this, cx| {
            let domain = domain.await.ok()?;
            if user_id.as_deref() != Some(domain.account_id.as_str())
                || account_epoch.as_deref() != Some(domain.account_epoch.as_str())
                || !SaiAccountCacheScope::is_current_account_epoch(user_id.as_deref(), account_epoch.as_deref())
                || !Self::still_current(&this, BLOCKED_ACCOUNT_APPEAL_SURFACE, generation, cx)
            {
                return None;
            }
            let repository = domain.repository.clone();
            let asked = block.clone();
            let status = Self::on_account_runtime(async move { repository.blocked_account_appeal_status(&asked).await })
                .await
                .ok()?
                .ok()?;
            let shown = cx.update(|cx| {
                Self::is_active_domain(&domain, cx)
                    && Self::read_presentation(cx, |presentation| {
                        presentation.active_full_block().map(|active| active.id()) == Some(block.id())
                    })
            });
            if !SaiAccountCacheScope::is_current_account_epoch(user_id.as_deref(), account_epoch.as_deref())
                || !shown
                || !Self::still_current(&this, BLOCKED_ACCOUNT_APPEAL_SURFACE, generation, cx)
            {
                return None;
            }
            status
        })
    }

    pub fn ensure_my_appeals_loaded(&mut self, cx: &mut Context<Self>) {
        if Self::read_presentation(cx, |presentation| presentation.is_stale()) {
            Self::update_presentation(cx, |presentation, cx| presentation.clear_stale(cx));
            self.load_my_appeals(true, cx);
            return;
        }
        let loads = Self::read_presentation(cx, |presentation| {
            let state = presentation.my_appeals_state();
            *state == ModerationLoadState::Idle || state.error_text().is_some()
        });
        if !loads {
            return;
        }
        self.load_my_appeals(false, cx);
    }

    pub fn refresh_my_appeals(&mut self, cx: &mut Context<Self>) {
        self.load_my_appeals(true, cx);
    }

    pub fn set_my_appeals_filter(&mut self, filter: AppealStatusFilter, cx: &mut Context<Self>) {
        if Self::read_presentation(cx, |presentation| presentation.my_appeals_filter() == filter) {
            return;
        }
        Self::update_presentation(cx, |presentation, cx| presentation.set_my_appeals_filter(filter, cx));
        self.load_my_appeals(false, cx);
    }

    fn load_my_appeals(&mut self, force: bool, cx: &mut Context<Self>) {
        let status = Self::read_presentation(cx, |presentation| presentation.my_appeals_filter().wire_value());
        let generation = self.next_generation(MY_APPEALS_SURFACE);
        if Self::read_presentation(cx, |presentation| presentation.my_appeals().is_empty()) {
            Self::update_presentation(cx, |presentation, cx| {
                presentation.set_my_appeals_state(ModerationLoadState::Loading, cx)
            });
        }
        let domain = Self::domain(cx);
        cx.spawn(async move |this, cx| {
            let result: anyhow::Result<()> = async {
                let domain = domain.await?;
                let repository = domain.repository.clone();
                let empty = cx.update(|cx| Self::read_presentation(cx, |presentation| presentation.my_appeals().is_empty()));
                if !force && empty {
                    let cached_repository = repository.clone();
                    let cached = Self::on_account_runtime(async move { cached_repository.cached_my_appeals(status).await })
                        .await
                        .ok()
                        .flatten();
                    if let Some(cached) = cached {
                        if !Self::still_current(&this, MY_APPEALS_SURFACE, generation, cx) {
                            return anyhow::Ok(());
                        }
                        cx.update(|cx| {
                            Self::update_presentation(cx, |presentation, cx| {
                                presentation.apply_my_appeals(cached.value, false, cx)
                            })
                        });
                    }
                }
                let page = Self::on_account_runtime(async move { repository.my_appeals(status, None).await }).await??;
                if !Self::still_current(&this, MY_APPEALS_SURFACE, generation, cx) {
                    return anyhow::Ok(());
                }
                cx.update(|cx| Self::update_presentation(cx, |presentation, cx| presentation.apply_my_appeals(page, false, cx)));
                anyhow::Ok(())
            }
            .await;
            if let Err(error) = result {
                if !Self::still_current(&this, MY_APPEALS_SURFACE, generation, cx) {
                    return;
                }
                let message = RiseModerationFailure::message(&error);
                let diagnostic = RiseModerationFailure::diagnostic(&error);
                cx.update(|cx| {
                    Self::update_presentation(cx, |presentation, cx| presentation.fail_my_appeals(message, cx));
                    Sai::log("Blocks", move || format!("getMyAppeals error: {diagnostic}"), SaiLogKind::Network, cx);
                });
            }
        })
        .detach();
    }

    pub fn load_more_my_appeals(&mut self, cx: &mut Context<Self>) {
        let (has_more, is_loading_more, cursor, status) = Self::read_presentation(cx, |presentation| {
            (
                presentation.my_appeals_has_more(),
                presentation.is_loading_more_my_appeals(),
                presentation.my_appeals_cursor(),
                presentation.my_appeals_filter().wire_value(),
            )
        });
        if !has_more || is_loading_more {
            return;
        }
        let Some(cursor) = cursor else {
            return;
        };
        let generation = self.current_generation(MY_APPEALS_SURFACE);
        Self::update_presentation(cx, |presentation, cx| presentation.set_loading_more_my_appeals(true, cx));
        let domain = Self::domain(cx);
        cx.spawn(async move |this, cx| {
            let result: anyhow::Result<_> = async {
                let domain = domain.await?;
                let repository = domain.repository.clone();
                Self::on_account_runtime(async move { repository.my_appeals(status, Some(cursor)).await }).await?
            }
            .await;
            if !Self::still_current(&this, MY_APPEALS_SURFACE, generation, cx) {
                return;
            }
            cx.update(|cx| match result {
                Ok(page) => Self::update_presentation(cx, |presentation, cx| presentation.apply_my_appeals(page, true, cx)),
                Err(error) => Self::update_presentation(cx, |presentation, cx| {
                    presentation.fail_my_appeals(RiseModerationFailure::message(&error), cx)
                }),
            });
        })
        .detach();
    }

    pub fn ensure_queue_loaded(&mut self, cx: &mut Context<Self>) {
        if Self::read_presentation(cx, |presentation| presentation.is_stale()) {
            Self::update_presentation(cx, |presentation, cx| presentation.clear_stale(cx));
            self.load_queue(true, false, cx);
            return;
        }
        let loads = Self::read_presentation(cx, |presentation| {
            let state = presentation.queue_state();
            *state == ModerationLoadState::Idle || state.error_text().is_some()
        });
        if !loads {
            return;
        }
        self.load_queue(false, false, cx);
    }

    pub fn open_queue(&mut self, cx: &mut Context<Self>) {
        let fresh = Self::read_presentation(cx, |presentation| {
            *presentation.queue_state() == ModerationLoadState::Loaded && !presentation.is_stale()
        });
        if !fresh {
            self.ensure_queue_loaded(cx);
            return;
        }
        self.load_queue(true, true, cx);
    }

    pub fn refresh_queue(&mut self, cx: &mut Context<Self>) {
        self.load_queue(true, false, cx);
    }

    pub fn set_queue_status_filter(&mut self, status: ReportStatus, cx: &mut Context<Self>) {
        let mut filter = Self::read_presentation(cx, |presentation| presentation.queue_filter().clone());
        if filter.status == status {
            return;
        }
        filter.status = status;
        Self::update_presentation(cx, |presentation, cx| presentation.set_queue_filter(filter, cx));
        self.load_queue(false, false, cx);
    }

    pub fn set_queue_type_filter(&mut self, r#type: Option<ReportTargetType>, cx: &mut Context<Self>) {
        let mut filter = Self::read_presentation(cx, |presentation| presentation.queue_filter().clone());
        if filter.r#type == r#type {
            return;
        }
        filter.r#type = r#type;
        Self::update_presentation(cx, |presentation, cx| presentation.set_queue_filter(filter, cx));
        self.load_queue(false, false, cx);
    }

    pub fn set_queue_sort(&mut self, sort: ReportQueueSort, cx: &mut Context<Self>) {
        let mut filter = Self::read_presentation(cx, |presentation| presentation.queue_filter().clone());
        if filter.sort == sort {
            return;
        }
        filter.sort = sort;
        Self::update_presentation(cx, |presentation, cx| presentation.set_queue_filter(filter, cx));
        self.load_queue(false, false, cx);
    }

    pub fn set_queue_target(&mut self, user_id: Option<&str>, cx: &mut Context<Self>) {
        let mut filter = Self::read_presentation(cx, |presentation| presentation.queue_filter().clone());
        let next = user_id
            .map(trimming_whitespaces_and_newlines)
            .filter(|normalized| !normalized.is_empty())
            .map(|normalized| normalized.to_owned());
        if filter.target_user_id == next {
            return;
        }
        filter.target_user_id = next;
        Self::update_presentation(cx, |presentation, cx| presentation.set_queue_filter(filter, cx));
        self.load_queue(false, false, cx);
    }

    fn load_queue(&mut self, force: bool, keep_loaded_tail: bool, cx: &mut Context<Self>) {
        let filter = Self::read_presentation(cx, |presentation| presentation.queue_filter().clone());
        let generation = self.next_generation(QUEUE_SURFACE);
        if Self::read_presentation(cx, |presentation| presentation.queue().is_empty()) {
            Self::update_presentation(cx, |presentation, cx| presentation.set_queue_state(ModerationLoadState::Loading, cx));
        }
        let domain = Self::domain(cx);
        cx.spawn(async move |this, cx| {
            let result: anyhow::Result<()> = async {
                let domain = domain.await?;
                let repository = domain.repository.clone();
                let empty = cx.update(|cx| Self::read_presentation(cx, |presentation| presentation.queue().is_empty()));
                if !force && empty {
                    let cached_repository = repository.clone();
                    let cached_filter = filter.clone();
                    let cached =
                        Self::on_account_runtime(async move { cached_repository.cached_queue(&cached_filter).await })
                            .await
                            .ok()
                            .flatten();
                    if let Some(cached) = cached {
                        if !Self::still_current(&this, QUEUE_SURFACE, generation, cx) {
                            return anyhow::Ok(());
                        }
                        cx.update(|cx| {
                            Self::update_presentation(cx, |presentation, cx| presentation.apply_queue(cached.value, false, cx))
                        });
                    }
                }
                let page = Self::on_account_runtime(async move { repository.queue(&filter, None).await }).await??;
                if !Self::still_current(&this, QUEUE_SURFACE, generation, cx) {
                    return anyhow::Ok(());
                }
                cx.update(|cx| {
                    Self::update_presentation(cx, |presentation, cx| {
                        if keep_loaded_tail {
                            presentation.apply_queue_head(page, cx);
                        } else {
                            presentation.apply_queue(page, false, cx);
                        }
                    })
                });
                anyhow::Ok(())
            }
            .await;
            if let Err(error) = result {
                if !Self::still_current(&this, QUEUE_SURFACE, generation, cx) {
                    return;
                }
                let message = RiseModerationFailure::message(&error);
                let diagnostic = RiseModerationFailure::diagnostic(&error);
                cx.update(|cx| {
                    Self::update_presentation(cx, |presentation, cx| presentation.fail_queue(message, cx));
                    Sai::log("Report", move || format!("getReports error: {diagnostic}"), SaiLogKind::Network, cx);
                });
            }
        })
        .detach();
    }

    pub fn load_more_queue(&mut self, cx: &mut Context<Self>) {
        let (has_more, is_loading_more, cursor, filter) = Self::read_presentation(cx, |presentation| {
            (
                presentation.queue_has_more(),
                presentation.is_loading_more_queue(),
                presentation.queue_cursor(),
                presentation.queue_filter().clone(),
            )
        });
        if !has_more || is_loading_more {
            return;
        }
        let Some(cursor) = cursor else {
            return;
        };
        let generation = self.current_generation(QUEUE_SURFACE);
        Self::update_presentation(cx, |presentation, cx| presentation.set_loading_more_queue(true, cx));
        let domain = Self::domain(cx);
        cx.spawn(async move |this, cx| {
            let result: anyhow::Result<_> = async {
                let domain = domain.await?;
                let repository = domain.repository.clone();
                Self::on_account_runtime(async move { repository.queue(&filter, Some(cursor)).await }).await?
            }
            .await;
            if !Self::still_current(&this, QUEUE_SURFACE, generation, cx) {
                return;
            }
            cx.update(|cx| match result {
                Ok(page) => Self::update_presentation(cx, |presentation, cx| presentation.apply_queue(page, true, cx)),
                Err(error) => Self::update_presentation(cx, |presentation, cx| {
                    presentation.fail_queue(RiseModerationFailure::message(&error), cx)
                }),
            });
        })
        .detach();
    }

    pub fn open_case(&mut self, report_id: i64, cx: &mut Context<Self>) {
        let surface = format!("case:{report_id}");
        let generation = self.next_generation(&surface);
        if Self::read_presentation(cx, |presentation| !presentation.cases().contains_key(&report_id)) {
            Self::update_presentation(cx, |presentation, cx| {
                presentation.set_case_state(report_id, ModerationLoadState::Loading, cx)
            });
        }
        let domain = Self::domain(cx);
        cx.spawn(async move |this, cx| {
            let result: anyhow::Result<()> = async {
                let domain = domain.await?;
                let repository = domain.repository.clone();
                let missing =
                    cx.update(|cx| Self::read_presentation(cx, |presentation| !presentation.cases().contains_key(&report_id)));
                if missing {
                    let cached_repository = repository.clone();
                    let cached = Self::on_account_runtime(async move { cached_repository.cached_case(report_id).await })
                        .await
                        .ok()
                        .flatten();
                    if let Some(cached) = cached {
                        if !Self::still_current(&this, &surface, generation, cx) {
                            return anyhow::Ok(());
                        }
                        cx.update(|cx| Self::update_presentation(cx, |presentation, cx| presentation.apply_case(cached.value, cx)));
                    }
                }
                let value = Self::on_account_runtime(async move { repository.report_case(report_id).await }).await??;
                if !Self::still_current(&this, &surface, generation, cx) {
                    return anyhow::Ok(());
                }
                cx.update(|cx| Self::update_presentation(cx, |presentation, cx| presentation.apply_case(value, cx)));
                anyhow::Ok(())
            }
            .await;
            if let Err(error) = result {
                if !Self::still_current(&this, &surface, generation, cx) {
                    return;
                }
                let message = RiseModerationFailure::message(&error);
                let diagnostic = RiseModerationFailure::diagnostic(&error);
                cx.update(|cx| {
                    Self::update_presentation(cx, |presentation, cx| presentation.fail_case(report_id, message, cx));
                    Sai::log(
                        "Report",
                        move || format!("getReportCase({report_id}) error: {diagnostic}"),
                        SaiLogKind::Network,
                        cx,
                    );
                });
            }
        })
        .detach();
    }

    pub fn refresh_case(&mut self, report_id: i64, cx: &mut Context<Self>) {
        self.open_case(report_id, cx);
    }

    pub fn resolve(
        &mut self,
        report_id: i64,
        action: ResolveAction,
        decision: &str,
        block_type: Option<ModerationMuteScope>,
        duration_days: Option<i64>,
        note: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        if Self::read_presentation(cx, |presentation| presentation.resolving_reports().contains(&report_id)) {
            return Task::ready(false);
        }
        Self::update_presentation(cx, |presentation, cx| presentation.set_resolving(report_id, true, cx));
        let trimmed = trimming_whitespaces_and_newlines(decision);
        let mut request = rise_canon::rise_request::report::ResolveReport {
            action: Some(action.wire().to_owned()),
            decision: Some(if trimmed.is_empty() { tr(action.title_key()) } else { trimmed.to_owned() }),
            report_id: Some(report_id),
            ..rise_canon::rise_request::report::ResolveReport::default()
        };
        match action.block_type_rule() {
            BlockTypeRule::AlwaysFull => {
                request.block_type = Some(UserBlockType::Full.wire().to_owned());
            }
            BlockTypeRule::OptionalFullDefault => {
                request.block_type = Some(block_type.unwrap_or(UserBlockType::Full).wire().to_owned());
            }
            BlockTypeRule::RequiredPartial => {
                request.block_type = Some(block_type.unwrap_or(UserBlockType::Post).wire().to_owned());
            }
            BlockTypeRule::Forbidden => {}
        }
        if action.takes_duration()
            && let Some(duration_days) = duration_days
        {
            request.block_duration_days = Some(duration_days);
        }
        let trimmed_note = note.map(trimming_whitespaces_and_newlines).unwrap_or_default();
        if !trimmed_note.is_empty() {
            request.note = Some(trimmed_note.to_owned());
        }
        let domain = Self::domain(cx);
        cx.spawn(async move |this, cx| {
            let result: anyhow::Result<_> = async {
                let domain = domain.await?;
                let repository = domain.repository.clone();
                Self::on_account_runtime(async move { repository.resolve_report(request).await }).await?
            }
            .await;
            let resolved = match result {
                Ok(report) => {
                    cx.update(|cx| {
                        Self::update_presentation(cx, |presentation, cx| presentation.replace_report(report, cx));
                        notification(NotificationKind::Success);
                        Self::send_toast(
                            tr("moderation_resolve_success_title"),
                            tr("moderation_resolve_success_content"),
                            NotifierType::Success,
                            "checkmark.seal.fill",
                            cx,
                        );
                    });
                    let _ = this.update(cx, |this, cx| this.open_case(report_id, cx));
                    true
                }
                Err(error) => {
                    let message = RiseModerationFailure::message(&error);
                    let diagnostic = RiseModerationFailure::diagnostic(&error);
                    cx.update(|cx| {
                        Self::send_toast(
                            tr("moderation_resolve_failed_title"),
                            message,
                            NotifierType::Warning,
                            "exclamationmark.triangle.fill",
                            cx,
                        );
                        Sai::log("Report", move || format!("resolveReport error: {diagnostic}"), SaiLogKind::Network, cx);
                    });
                    false
                }
            };
            cx.update(|cx| Self::update_presentation(cx, |presentation, cx| presentation.set_resolving(report_id, false, cx)));
            resolved
        })
    }

    pub fn add_note(
        &mut self,
        report_id: i64,
        text: &str,
        attachments: Vec<BugReportAttachment>,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        let trimmed = trimming_whitespaces_and_newlines(text).to_owned();
        if trimmed.is_empty() && attachments.is_empty() {
            return Task::ready(false);
        }
        if Self::read_presentation(cx, |presentation| presentation.noting_reports().contains(&report_id)) {
            return Task::ready(false);
        }
        let owner = RiseModerationSubmissionOwner::new(cx);
        Self::update_presentation(cx, |presentation, cx| presentation.set_noting(report_id, true, cx));
        cx.spawn(async move |_, cx| {
            let mut owner = owner;
            let result = Self::add_note_body(&mut owner, report_id, trimmed, attachments, cx).await;
            let noted = match result {
                Ok(noted) => noted,
                Err(error) => {
                    if cx.update(|cx| owner.is_current(cx)) {
                        let message = RiseModerationFailure::message(&error);
                        cx.update(|cx| {
                            Self::send_toast(
                                tr("case_note_failed_title"),
                                message,
                                NotifierType::Warning,
                                "exclamationmark.triangle.fill",
                                cx,
                            )
                        });
                    }
                    false
                }
            };
            cx.update(|cx| {
                if owner.is_current(cx) {
                    Self::update_presentation(cx, |presentation, cx| presentation.set_noting(report_id, false, cx));
                }
            });
            noted
        })
    }

    async fn add_note_body(
        owner: &mut RiseModerationSubmissionOwner,
        report_id: i64,
        trimmed: String,
        attachments: Vec<BugReportAttachment>,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<bool> {
        *owner = owner.clone().prepared(cx).await?;
        let Some(domain) = owner.domain.clone() else {
            return Ok(false);
        };
        let mut hasher = DefaultHasher::new();
        trimmed.hash(&mut hasher);
        let draft = NoteDraftPayload {
            draft_id: format!("note_{report_id}_{}", hasher.finish() as i64),
            report_id,
            text: trimmed,
            attachments,
        };
        let note = RiseModerationSubmitter::shared().submit_note(draft, owner.clone(), cx).await?;
        if !cx.update(|cx| owner.is_current(cx)) {
            return Ok(false);
        }
        let value = cx.update(|cx| {
            Self::update_presentation(cx, |presentation, cx| presentation.append_note(note, cx));
            Self::read_presentation(cx, |presentation| presentation.cases().get(&report_id).cloned())
        });
        if let Some(value) = value {
            let repository = domain.repository.clone();
            let _ = Self::on_account_runtime(async move { repository.store_case(&value).await }).await;
        }
        Ok(cx.update(|cx| owner.is_current(cx)))
    }

    pub fn reopen(&mut self, report_id: i64, reason: &str, cx: &mut Context<Self>) -> Task<bool> {
        if Self::read_presentation(cx, |presentation| presentation.reopening_reports().contains(&report_id)) {
            return Task::ready(false);
        }
        Self::update_presentation(cx, |presentation, cx| presentation.set_reopening(report_id, true, cx));
        let trimmed = trimming_whitespaces_and_newlines(reason);
        let request = rise_canon::rise_request::report::ReopenReport {
            reason: (!trimmed.is_empty()).then(|| trimmed.to_owned()),
            report_id: Some(report_id),
        };
        let domain = Self::domain(cx);
        cx.spawn(async move |this, cx| {
            let result: anyhow::Result<_> = async {
                let domain = domain.await?;
                let repository = domain.repository.clone();
                Self::on_account_runtime(async move { repository.reopen_report(request).await }).await?
            }
            .await;
            let reopened = match result {
                Ok(report) => {
                    cx.update(|cx| {
                        Self::update_presentation(cx, |presentation, cx| presentation.replace_report(report, cx));
                        notification(NotificationKind::Success);
                        Self::send_toast(
                            tr("case_reopened_title"),
                            tr("case_reopened_content"),
                            NotifierType::Success,
                            "arrow.counterclockwise.circle.fill",
                            cx,
                        );
                    });
                    let _ = this.update(cx, |this, cx| {
                        this.open_case(report_id, cx);
                        this.refresh_queue(cx);
                    });
                    true
                }
                Err(error) => {
                    let message = RiseModerationFailure::message(&error);
                    cx.update(|cx| {
                        Self::send_toast(
                            tr("case_reopen_failed_title"),
                            message,
                            NotifierType::Warning,
                            "exclamationmark.triangle.fill",
                            cx,
                        )
                    });
                    false
                }
            };
            cx.update(|cx| Self::update_presentation(cx, |presentation, cx| presentation.set_reopening(report_id, false, cx)));
            reopened
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn admin_sanction(
        &mut self,
        kind: AdminSanctionKind,
        target_user_id: Option<&str>,
        block_type: Option<UserBlockType>,
        duration_days: Option<i64>,
        ip: Option<&str>,
        reason: Option<&str>,
        report_id: Option<i64>,
        appeal_id: Option<i64>,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        if Self::read_presentation(cx, |presentation| presentation.is_sanctioning()) {
            return Task::ready(false);
        }
        Self::update_presentation(cx, |presentation, cx| presentation.set_sanctioning(true, cx));
        let trimmed_reason = reason.map(trimming_whitespaces_and_newlines).unwrap_or_default();
        let request = rise_canon::rise_request::report::AdminSanction {
            kind: Some(kind.raw_value().to_owned()),
            target_user_id: target_user_id.map(|user_id| trimming_whitespaces_and_newlines(user_id).to_owned()),
            block_type: kind
                .takes_block_type()
                .then(|| block_type.unwrap_or(UserBlockType::Full).wire().to_owned()),
            duration_days: kind.takes_duration().then(|| duration_days.unwrap_or(0)),
            ip: if kind.takes_ip() { ip.map(|ip| trimming_whitespaces_and_newlines(ip).to_owned()) } else { None },
            reason: (!trimmed_reason.is_empty()).then(|| trimmed_reason.to_owned()),
            report_id,
            appeal_id,
        };
        let domain = Self::domain(cx);
        cx.spawn(async move |this, cx| {
            let result: anyhow::Result<_> = async {
                let domain = domain.await?;
                let repository = domain.repository.clone();
                Self::on_account_runtime(async move { repository.admin_sanction(request).await }).await?
            }
            .await;
            let applied = match result {
                Ok(_) => {
                    cx.update(|cx| {
                        notification(NotificationKind::Success);
                        Self::send_toast(
                            tr("admin_sanction_applied_title"),
                            tr(kind.title_key()),
                            NotifierType::Success,
                            "checkmark.shield.fill",
                            cx,
                        );
                    });
                    let _ = this.update(cx, |this, cx| {
                        if let Some(report_id) = report_id {
                            this.open_case(report_id, cx);
                        }
                        if let Some(appeal_id) = appeal_id {
                            this.open_appeal_case(appeal_id, cx);
                        }
                    });
                    true
                }
                Err(error) => {
                    let message = RiseModerationFailure::message(&error);
                    let diagnostic = RiseModerationFailure::diagnostic(&error);
                    let raw = kind.raw_value();
                    cx.update(|cx| {
                        Self::send_toast(
                            tr("admin_sanction_failed_title"),
                            message,
                            NotifierType::Warning,
                            "exclamationmark.triangle.fill",
                            cx,
                        );
                        Sai::log(
                            "Report",
                            move || format!("adminSanction({raw}) error: {diagnostic}"),
                            SaiLogKind::Network,
                            cx,
                        );
                    });
                    false
                }
            };
            cx.update(|cx| Self::update_presentation(cx, |presentation, cx| presentation.set_sanctioning(false, cx)));
            applied
        })
    }

    pub fn ensure_review_loaded(&mut self, cx: &mut Context<Self>) {
        if Self::read_presentation(cx, |presentation| presentation.is_stale()) {
            Self::update_presentation(cx, |presentation, cx| presentation.clear_stale(cx));
            self.load_review(true, cx);
            return;
        }
        let loads = Self::read_presentation(cx, |presentation| {
            let state = presentation.review_state();
            *state == ModerationLoadState::Idle || state.error_text().is_some()
        });
        if !loads {
            return;
        }
        self.load_review(false, cx);
    }

    pub fn refresh_review(&mut self, cx: &mut Context<Self>) {
        self.load_review(true, cx);
    }

    pub fn set_review_filter(&mut self, filter: AppealReviewFilter, cx: &mut Context<Self>) {
        if Self::read_presentation(cx, |presentation| presentation.review_filter() == filter) {
            return;
        }
        Self::update_presentation(cx, |presentation, cx| presentation.set_review_filter(filter, cx));
        self.load_review(false, cx);
    }

    fn load_review(&mut self, force: bool, cx: &mut Context<Self>) {
        let status = Self::read_presentation(cx, |presentation| presentation.review_filter().wire_value());
        let generation = self.next_generation(REVIEW_SURFACE);
        if Self::read_presentation(cx, |presentation| presentation.review().is_empty()) {
            Self::update_presentation(cx, |presentation, cx| presentation.set_review_state(ModerationLoadState::Loading, cx));
        }
        let domain = Self::domain(cx);
        cx.spawn(async move |this, cx| {
            let result: anyhow::Result<()> = async {
                let domain = domain.await?;
                let repository = domain.repository.clone();
                let empty = cx.update(|cx| Self::read_presentation(cx, |presentation| presentation.review().is_empty()));
                if !force && empty {
                    let cached_repository = repository.clone();
                    let cached = Self::on_account_runtime(async move { cached_repository.cached_review(status).await })
                        .await
                        .ok()
                        .flatten();
                    if let Some(cached) = cached {
                        if !Self::still_current(&this, REVIEW_SURFACE, generation, cx) {
                            return anyhow::Ok(());
                        }
                        cx.update(|cx| {
                            Self::update_presentation(cx, |presentation, cx| presentation.apply_review(cached.value, false, cx))
                        });
                    }
                }
                let page = Self::on_account_runtime(async move { repository.review(status, None).await }).await??;
                if !Self::still_current(&this, REVIEW_SURFACE, generation, cx) {
                    return anyhow::Ok(());
                }
                cx.update(|cx| Self::update_presentation(cx, |presentation, cx| presentation.apply_review(page, false, cx)));
                anyhow::Ok(())
            }
            .await;
            if let Err(error) = result {
                if !Self::still_current(&this, REVIEW_SURFACE, generation, cx) {
                    return;
                }
                let message = if RiseModerationFailure::is_access_denied(&error) {
                    tr("appeals_review_no_access")
                } else {
                    RiseModerationFailure::message(&error)
                };
                let diagnostic = RiseModerationFailure::diagnostic(&error);
                cx.update(|cx| {
                    Self::update_presentation(cx, |presentation, cx| presentation.fail_review(message, cx));
                    Sai::log("Appeals", move || format!("getAppeals error: {diagnostic}"), SaiLogKind::Network, cx);
                });
            }
        })
        .detach();
    }

    pub fn load_more_review(&mut self, cx: &mut Context<Self>) {
        let (has_more, is_loading_more, cursor, status) = Self::read_presentation(cx, |presentation| {
            (
                presentation.review_has_more(),
                presentation.is_loading_more_review(),
                presentation.review_cursor(),
                presentation.review_filter().wire_value(),
            )
        });
        if !has_more || is_loading_more {
            return;
        }
        let Some(cursor) = cursor else {
            return;
        };
        let generation = self.current_generation(REVIEW_SURFACE);
        Self::update_presentation(cx, |presentation, cx| presentation.set_loading_more_review(true, cx));
        let domain = Self::domain(cx);
        cx.spawn(async move |this, cx| {
            let result: anyhow::Result<_> = async {
                let domain = domain.await?;
                let repository = domain.repository.clone();
                Self::on_account_runtime(async move { repository.review(status, Some(cursor)).await }).await?
            }
            .await;
            if !Self::still_current(&this, REVIEW_SURFACE, generation, cx) {
                return;
            }
            cx.update(|cx| match result {
                Ok(page) => Self::update_presentation(cx, |presentation, cx| presentation.apply_review(page, true, cx)),
                Err(error) => Self::update_presentation(cx, |presentation, cx| {
                    presentation.fail_review(RiseModerationFailure::message(&error), cx)
                }),
            });
        })
        .detach();
    }

    pub fn open_appeal_case(&mut self, appeal_id: i64, cx: &mut Context<Self>) {
        let surface = format!("appeal:{appeal_id}");
        let generation = self.next_generation(&surface);
        if Self::read_presentation(cx, |presentation| !presentation.appeal_cases().contains_key(&appeal_id)) {
            Self::update_presentation(cx, |presentation, cx| {
                presentation.set_appeal_case_state(appeal_id, ModerationLoadState::Loading, cx)
            });
        }
        let domain = Self::domain(cx);
        cx.spawn(async move |this, cx| {
            let result: anyhow::Result<()> = async {
                let domain = domain.await?;
                let repository = domain.repository.clone();
                let missing = cx.update(|cx| {
                    Self::read_presentation(cx, |presentation| !presentation.appeal_cases().contains_key(&appeal_id))
                });
                if missing {
                    let cached_repository = repository.clone();
                    let cached =
                        Self::on_account_runtime(async move { cached_repository.cached_appeal_case(appeal_id).await })
                            .await
                            .ok()
                            .flatten();
                    if let Some(cached) = cached {
                        if !Self::still_current(&this, &surface, generation, cx) {
                            return anyhow::Ok(());
                        }
                        cx.update(|cx| {
                            Self::update_presentation(cx, |presentation, cx| presentation.apply_appeal_case(cached.value, cx))
                        });
                    }
                }
                let value = Self::on_account_runtime(async move { repository.appeal_case(appeal_id).await }).await??;
                if !Self::still_current(&this, &surface, generation, cx) {
                    return anyhow::Ok(());
                }
                cx.update(|cx| Self::update_presentation(cx, |presentation, cx| presentation.apply_appeal_case(value, cx)));
                anyhow::Ok(())
            }
            .await;
            if let Err(error) = result {
                if !Self::still_current(&this, &surface, generation, cx) {
                    return;
                }
                let message = RiseModerationFailure::message(&error);
                let diagnostic = RiseModerationFailure::diagnostic(&error);
                cx.update(|cx| {
                    Self::update_presentation(cx, |presentation, cx| presentation.fail_appeal_case(appeal_id, message, cx));
                    Sai::log(
                        "Appeals",
                        move || format!("getAppealCase({appeal_id}) error: {diagnostic}"),
                        SaiLogKind::Network,
                        cx,
                    );
                });
            }
        })
        .detach();
    }

    pub fn resolve_appeal(
        &mut self,
        appeal_id: i64,
        decision: AppealDecision,
        decision_text: &str,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        if Self::read_presentation(cx, |presentation| presentation.resolving_appeals().contains(&appeal_id)) {
            return Task::ready(false);
        }
        Self::update_presentation(cx, |presentation, cx| presentation.set_resolving_appeal(appeal_id, true, cx));
        let trimmed = trimming_whitespaces_and_newlines(decision_text);
        let request = rise_canon::rise_request::report::ResolveAppeal {
            appeal_id: Some(appeal_id),
            decision: Some(decision.wire().to_owned()),
            decision_text: (!trimmed.is_empty()).then(|| trimmed.to_owned()),
        };
        let domain = Self::domain(cx);
        cx.spawn(async move |this, cx| {
            let result: anyhow::Result<_> = async {
                let domain = domain.await?;
                let repository = domain.repository.clone();
                Self::on_account_runtime(async move { repository.resolve_appeal(request).await }).await?
            }
            .await;
            let resolved = match result {
                Ok(appeal) => {
                    let accepted = decision == AppealDecision::Accept;
                    cx.update(|cx| {
                        if let Some(appeal) = appeal {
                            Self::update_presentation(cx, |presentation, cx| presentation.replace_appeal(appeal, cx));
                        }
                        notification(NotificationKind::Success);
                        Self::send_toast(
                            if accepted { tr("appeal_review_accepted_title") } else { tr("appeal_review_rejected_title") },
                            if accepted { tr("appeal_review_accepted_content") } else { tr("appeal_review_rejected_content") },
                            NotifierType::Success,
                            decision.icon_name(),
                            cx,
                        );
                    });
                    let _ = this.update(cx, |this, cx| this.open_appeal_case(appeal_id, cx));
                    true
                }
                Err(error) => {
                    let message = RiseModerationFailure::message(&error);
                    let diagnostic = RiseModerationFailure::diagnostic(&error);
                    cx.update(|cx| {
                        Self::send_toast(
                            tr("appeal_review_failed_title"),
                            message,
                            NotifierType::Warning,
                            "exclamationmark.triangle.fill",
                            cx,
                        );
                        Sai::log(
                            "Appeals",
                            move || format!("resolveAppeal({appeal_id}) error: {diagnostic}"),
                            SaiLogKind::Network,
                            cx,
                        );
                    });
                    false
                }
            };
            cx.update(|cx| {
                Self::update_presentation(cx, |presentation, cx| presentation.set_resolving_appeal(appeal_id, false, cx))
            });
            resolved
        })
    }

    pub fn handle_report_resolved(&mut self, payload: Option<&Value>, cx: &mut Context<Self>) {
        if Self::read_presentation(cx, |presentation| *presentation.my_reports_state() == ModerationLoadState::Loaded) {
            let applied = Self::event_resolution(payload)
                .map(|resolution| {
                    Self::update_presentation(cx, |presentation, cx| presentation.apply_my_report_resolution(&resolution, cx))
                })
                .unwrap_or(false);
            if !applied {
                self.refresh_my_reports(cx);
            }
        }
        if let Some(report_id) = Self::event_report_id(payload)
            && Self::read_presentation(cx, |presentation| presentation.cases().contains_key(&report_id))
        {
            self.open_case(report_id, cx);
        }
        Self::send_toast(
            tr("report_resolved_title"),
            tr("report_resolved_content"),
            NotifierType::Info,
            "checkmark.seal.fill",
            cx,
        );
    }

    pub fn handle_report_reopened(&mut self, payload: Option<&Value>, cx: &mut Context<Self>) {
        if Self::read_presentation(cx, |presentation| *presentation.my_reports_state() == ModerationLoadState::Loaded) {
            self.refresh_my_reports(cx);
        }
        if let Some(report_id) = Self::event_report_id(payload)
            && Self::read_presentation(cx, |presentation| presentation.cases().contains_key(&report_id))
        {
            self.open_case(report_id, cx);
        }
        Self::send_toast(
            tr("report_reopened_title"),
            tr("report_reopened_content"),
            NotifierType::Info,
            "arrow.counterclockwise.circle.fill",
            cx,
        );
    }

    pub fn handle_appeal_resolved(&mut self, cx: &mut Context<Self>) {
        self.bump_blocked_account_appeal_revision(cx);
        if Self::read_presentation(cx, |presentation| *presentation.my_appeals_state() == ModerationLoadState::Loaded) {
            self.refresh_my_appeals(cx);
        }
        if Self::read_presentation(cx, |presentation| *presentation.my_blocks_state() == ModerationLoadState::Loaded) {
            self.refresh_blocks(cx);
        }
        Self::send_toast(
            tr("appeal_resolved_title"),
            tr("appeal_resolved_content"),
            NotifierType::Info,
            "checkmark.seal.fill",
            cx,
        );
    }

    pub fn handle_user_sanctioned(&mut self, cx: &mut Context<Self>) {
        if Self::read_presentation(cx, |presentation| *presentation.my_blocks_state() == ModerationLoadState::Loaded) {
            self.refresh_blocks(cx);
        }
    }

    pub fn handle_user_warned(&mut self, cx: &mut Context<Self>) {
        if Self::read_presentation(cx, |presentation| *presentation.my_blocks_state() == ModerationLoadState::Loaded) {
            self.refresh_blocks(cx);
        }
        notification(NotificationKind::Warning);
        Self::send_toast(
            tr("user_warned_title"),
            tr("user_warned_content"),
            NotifierType::Warning,
            "exclamationmark.triangle.fill",
            cx,
        );
    }

    pub fn refresh_loaded_surfaces(&mut self, cx: &mut Context<Self>) {
        let (my_reports, my_blocks, my_appeals, queue, review) = Self::read_presentation(cx, |presentation| {
            (
                *presentation.my_reports_state() == ModerationLoadState::Loaded,
                *presentation.my_blocks_state() == ModerationLoadState::Loaded,
                *presentation.my_appeals_state() == ModerationLoadState::Loaded,
                *presentation.queue_state() == ModerationLoadState::Loaded,
                *presentation.review_state() == ModerationLoadState::Loaded,
            )
        });
        if my_reports {
            self.refresh_my_reports(cx);
        }
        if my_blocks {
            self.refresh_blocks(cx);
        }
        if my_appeals {
            self.refresh_my_appeals(cx);
        }
        if queue {
            self.load_queue(true, true, cx);
        }
        if review {
            self.refresh_review(cx);
        }
        Self::update_presentation(cx, |presentation, cx| presentation.clear_stale(cx));
    }

    fn non_empty(value: Option<String>) -> Option<String> {
        value.filter(|value| !value.is_empty())
    }

    fn event_resolution(payload: Option<&Value>) -> Option<ReportResolutionEvent> {
        let Some(RiseEvent::ReportResolved(event)) = RiseEvent::decode_payload(name::REPORT_RESOLVED, payload) else {
            return None;
        };
        if event.report_id <= 0 || event.status.is_empty() {
            return None;
        }
        Some(ReportResolutionEvent {
            report_id: event.report_id,
            status: event.status.to_uppercase(),
            decision: Self::non_empty(Some(event.decision)),
            action: Self::non_empty(Some(event.action)),
            content_deleted: event.content_deleted,
            block_type: Self::non_empty(event.block_type),
            block_duration_days: event.block_duration_days,
        })
    }

    fn event_report_id(payload: Option<&Value>) -> Option<i64> {
        if let Some(RiseEvent::ReportResolved(event)) = RiseEvent::decode_payload(name::REPORT_RESOLVED, payload)
            && event.report_id > 0
        {
            return Some(event.report_id);
        }
        if let Some(RiseEvent::ReportReopened(event)) = RiseEvent::decode_payload(name::REPORT_REOPENED, payload)
            && event.report_id > 0
        {
            return Some(event.report_id);
        }
        None
    }

    pub fn ensure_moderator_profile_loaded(&mut self, cx: &mut Context<Self>) {
        if ModerationServicesStore::shared(cx).read(cx).did_load_moderator_profile {
            return;
        }
        let actions = ModerationActionsStore::shared(cx);
        if actions.read(cx).my_moderator_profile_sai.is_pending() {
            return;
        }
        actions.update(cx, |actions, cx| actions.get_my_moderator_profile_action(false, cx));
    }

    pub fn refresh_moderator_profile(&mut self, cx: &mut Context<Self>) {
        ModerationActionsStore::shared(cx).update(cx, |actions, cx| actions.get_my_moderator_profile_action(true, cx));
    }

    pub fn moderator_rights_changed(&mut self, cx: &mut Context<Self>) {
        ModerationActionsStore::shared(cx).update(cx, |actions, cx| actions.get_my_moderator_profile_action(true, cx));
        let (queue_idle, review_idle) = Self::read_presentation(cx, |presentation| {
            (
                *presentation.queue_state() == ModerationLoadState::Idle,
                *presentation.review_state() == ModerationLoadState::Idle,
            )
        });
        if !queue_idle {
            self.load_queue(true, false, cx);
        }
        if !review_idle {
            self.load_review(true, cx);
        }
        let open_reports = self.open_case_screens.keys().copied().collect::<Vec<_>>();
        for report_id in open_reports {
            if Self::read_presentation(cx, |presentation| presentation.cases().contains_key(&report_id)) {
                self.open_case(report_id, cx);
            }
        }
    }

    pub fn case_screen_opened(&mut self, report_id: i64) {
        *self.open_case_screens.entry(report_id).or_insert(0) += 1;
    }

    pub fn case_screen_closed(&mut self, report_id: i64) {
        let Some(count) = self.open_case_screens.get(&report_id).copied() else {
            return;
        };
        if count > 1 {
            self.open_case_screens.insert(report_id, count - 1);
        } else {
            self.open_case_screens.remove(&report_id);
        }
    }

    fn kind_filter(cx: &mut App) -> ModeratorKindFilter {
        ModerationServicesStore::shared(cx).read(cx).kind_filter
    }

    pub fn ensure_moderators_loaded(&mut self, cx: &mut Context<Self>) {
        if ModerationServicesStore::did_load_moderators(cx) && ModerationServicesStore::has_moderators_payload(cx) {
            return;
        }
        let actions = ModerationActionsStore::shared(cx);
        if actions.read(cx).all_moderators_sai.is_pending() {
            return;
        }
        let kind = Self::kind_filter(cx);
        actions.update(cx, |actions, cx| actions.get_all_moderators_action(None, kind, false, cx));
    }

    pub fn refresh_moderators(&mut self, cx: &mut Context<Self>) {
        let kind = Self::kind_filter(cx);
        ModerationActionsStore::shared(cx).update(cx, |actions, cx| actions.get_all_moderators_action(None, kind, true, cx));
    }

    pub fn load_more_moderators(&mut self, cx: &mut Context<Self>) {
        if !ModerationServicesStore::moderators_has_more(cx) {
            return;
        }
        let Some(anchor) = ModerationServicesStore::moderators_relative_id(cx) else {
            return;
        };
        let kind = Self::kind_filter(cx);
        let cache_key = ModerationServicesStore::moderators_cache_key(kind, cx);
        if !Self::can_request_continuation(&cache_key, cx) {
            return;
        }
        ModerationActionsStore::shared(cx)
            .update(cx, |actions, cx| actions.get_all_moderators_action(Some(anchor), kind, true, cx));
    }

    pub fn set_kind_filter(&mut self, kind: ModeratorKindFilter, cx: &mut Context<Self>) {
        if Self::kind_filter(cx) == kind {
            return;
        }
        ModerationServicesStore::shared(cx).update(cx, |services, cx| {
            services.kind_filter = kind;
            cx.notify();
        });
        self.refresh_moderators(cx);
    }

    pub fn load_detail(&mut self, user_id: &str, cx: &mut Context<Self>) {
        if user_id.is_empty() {
            return;
        }
        ModerationActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.get_moderator_profile_action(user_id, true, cx);
            actions.get_moderator_stats_action(user_id, true, cx);
        });
        self.ensure_history_loaded(user_id, cx);
    }

    pub fn refresh_detail(&mut self, user_id: &str, cx: &mut Context<Self>) {
        if user_id.is_empty() {
            return;
        }
        ModerationActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.get_moderator_profile_action(user_id, true, cx);
            actions.get_moderator_stats_action(user_id, true, cx);
            actions.get_moderator_history_action(user_id, None, true, cx);
        });
    }

    pub fn moderator(&self, user_id: &str, cx: &mut App) -> Option<ModeratorWithUserWireDTO> {
        ModerationServicesStore::moderator(user_id, cx)
    }

    pub fn stats(&self, user_id: &str, cx: &mut App) -> Option<ModeratorStatsWireDTO> {
        ModerationServicesStore::shared(cx).read(cx).stats(user_id)
    }

    pub fn ensure_history_loaded(&mut self, user_id: &str, cx: &mut Context<Self>) {
        if user_id.is_empty() {
            return;
        }
        if ModerationServicesStore::did_load_history(user_id, cx) {
            return;
        }
        let actions = ModerationActionsStore::shared(cx);
        if actions.read(cx).moderator_history_sai.is_pending() {
            return;
        }
        actions.update(cx, |actions, cx| actions.get_moderator_history_action(user_id, None, false, cx));
    }

    pub fn load_more_history(&mut self, user_id: &str, cx: &mut Context<Self>) {
        let detail = ModerationServicesStore::detail(user_id, cx);
        if !detail.history_has_more {
            return;
        }
        let Some(anchor) = detail.history_relative_id else {
            return;
        };
        let cache_key = ModerationServicesStore::moderator_history_cache_key(user_id, cx);
        if !Self::can_request_continuation(&cache_key, cx) {
            return;
        }
        ModerationActionsStore::shared(cx)
            .update(cx, |actions, cx| actions.get_moderator_history_action(user_id, Some(anchor), true, cx));
    }

    pub fn update_rights(
        &mut self,
        user_id: &str,
        permissions_json: &str,
        on_done: Option<ModerationActionDone>,
        cx: &mut Context<Self>,
    ) {
        if user_id.is_empty() {
            return;
        }
        let done: ModerationActionDone = Rc::new(move |cx: &mut App| {
            Self::shared(cx).update(cx, |this, cx| this.refresh_moderators(cx));
            if let Some(on_done) = &on_done {
                on_done(cx);
            }
        });
        ModerationActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.update_moderator_permissions_action(user_id, Some(permissions_json.to_owned()), None, Some(done), cx)
        });
    }

    pub fn set_active(&mut self, user_id: &str, is_active: bool, cx: &mut Context<Self>) {
        if user_id.is_empty() {
            return;
        }
        let Some(permissions_json) = self
            .moderator(user_id, cx)
            .and_then(|moderator| moderator.profile)
            .map(|profile| profile.permissions_json)
            .filter(|permissions_json| !permissions_json.is_empty())
        else {
            return;
        };
        let done: ModerationActionDone = Rc::new(|cx: &mut App| {
            Self::shared(cx).update(cx, |this, cx| this.refresh_moderators(cx));
        });
        ModerationActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.update_moderator_permissions_action(user_id, Some(permissions_json), Some(is_active), Some(done), cx)
        });
    }

    pub fn remove_moderator(&mut self, target_user_id: &str, on_done: Option<ModerationActionDone>, cx: &mut Context<Self>) {
        if target_user_id.is_empty() {
            return;
        }
        let done: ModerationActionDone = Rc::new(move |cx: &mut App| {
            Self::shared(cx).update(cx, |this, cx| this.refresh_moderators(cx));
            if let Some(on_done) = &on_done {
                on_done(cx);
            }
        });
        ModerationActionsStore::shared(cx)
            .update(cx, |actions, cx| actions.remove_moderator_action(target_user_id, Some(done), cx));
    }

    pub fn update_pending(&self, cx: &mut App) -> bool {
        ModerationActionsStore::shared(cx).read(cx).update_permissions_sai.is_pending()
    }
}
