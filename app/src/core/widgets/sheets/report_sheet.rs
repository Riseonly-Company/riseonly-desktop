use std::collections::HashMap;
use std::rc::Rc;
use std::sync::LazyLock;

use gpui::{
    AnyElement, App, Context, ElementId, Entity, ExternalPaths, FontWeight, Hsla, IntoElement, ParentElement, Render, RenderOnce,
    SharedString, Styled, Subscription, Window, black, div, px, transparent_black, white,
};
use rise_canon::RiseLimits;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_core::foundation::foundation_url::Url;
use rise_i18n::tr;
use rise_platform::uniform_type::UTType;
use rise_platform::haptics::{impact, ImpactStyle};
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::keyboard_dismiss::SheetConfirmSurface;
use rise_ui::IconUi;
use rise_ui::button_ui::ButtonUi;
use rise_ui::main_text::MainText;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::sheet_header_bar::SheetHeaderBar;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::minimum_scale_factor::MinimumScaleText;
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::swiftui::transition::{Edge, Presence, Transition};
use rise_ui::switch_ui::SwitchUi;
use rise_theme::FONT_FAMILY;

use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::clever_image_ui::{CleverImageUi, ContentMode};
use crate::core::ui::text_area_ui::TextAreaUi;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::modules::diagnostics::engine::core::rise_bug_report_upload::{BugReportAttachment, BugReportAttachmentKind};
use crate::modules::diagnostics::stores::diagnostics::diagnostics_interactions::bug_report_interactions::BugReportAttachmentFactory;
use crate::modules::media_picker::api::media_picker_configuration::MediaPickerConfiguration;
use crate::modules::media_picker::api::media_picker_result::{MediaPickerItem, MediaPickerItemKind, MediaPickerResult};
use crate::modules::media_picker::export::dropped_video_probe::DroppedVideoProbe;
use crate::modules::media_picker::ui::media_picker_sheet::MediaPickerSheet;
use crate::modules::moderation::components::moderation_kit_ui::ModerationTint;
use crate::modules::moderation::engine::core::rise_report_evidence_upload::RiseReportEvidenceUpload;
use crate::modules::moderation::engine::rise_moderation_submitter::{ReportDraftPayload, RiseModerationSubmitter};
use crate::modules::moderation::stores::moderation_actions::moderation_types::{ReportReason, ReportReasonExt, ReportTarget};
use crate::modules::moderation::stores::moderation_interactions::moderation_interactions::ModerationInteractionsStore;

/// which of the two surfaces is up, the sheet or its evidence picker
/// two bottom sheets dont stack so the store closes one before opening the other
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReportSheetRoute {
    #[default]
    Idle,
    Sheet,
    ClosingSheetForPicker,
    ClosingSheetForDismissal,
    Picker,
    ClosingPickerForSheet,
}

/// who the report is about, shown under the title
/// a report from a list row must say which row
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReportSubject {
    pub name: String,
    pub tag: String,
    pub avatar_url: String,
    pub user_id: Option<String>,
}

pub struct ReportSheetStore {
    route: ReportSheetRoute,
    target: Option<ReportTarget>,
    subject: Option<ReportSubject>,
    context_note: Option<String>,
    draft_id: String,
    pub selected_reason: Option<ReportReason>,
    pub description: String,
    attachments: Vec<BugReportAttachment>,
    pub include_context: bool,

    handed_off_draft_id: Option<String>,

    is_checking_connection: bool,
}

impl ReportSheetStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            route: ReportSheetRoute::Idle,
            target: None,
            subject: None,
            context_note: None,
            draft_id: Uuid::new_v4().uuid_string(),
            selected_reason: None,
            description: String::new(),
            attachments: Vec::new(),
            include_context: true,
            handed_off_draft_id: None,
            is_checking_connection: false,
        }
    }

    pub fn route(&self) -> ReportSheetRoute {
        self.route
    }

    pub fn target(&self) -> Option<&ReportTarget> {
        self.target.as_ref()
    }

    pub fn subject(&self) -> Option<&ReportSubject> {
        self.subject.as_ref()
    }

    pub fn draft_id(&self) -> &str {
        &self.draft_id
    }

    pub fn attachments(&self) -> &[BugReportAttachment] {
        &self.attachments
    }

    pub fn is_checking_connection(&self) -> bool {
        self.is_checking_connection
    }

    pub fn mounts_sheet(&self) -> bool {
        matches!(
            self.route,
            ReportSheetRoute::Sheet | ReportSheetRoute::ClosingSheetForPicker | ReportSheetRoute::ClosingSheetForDismissal
        )
    }

    pub fn is_sheet_presented(&self) -> bool {
        self.route == ReportSheetRoute::Sheet
    }

    pub fn mounts_picker(&self) -> bool {
        self.route == ReportSheetRoute::Picker || self.route == ReportSheetRoute::ClosingPickerForSheet
    }

    pub fn is_picker_presented(&self) -> bool {
        self.route == ReportSheetRoute::Picker
    }

    pub fn has_interactive_surface(&self) -> bool {
        self.route != ReportSheetRoute::Idle
    }

    /// for callers that only ask whether the sheet is up
    pub fn is_visible(&self) -> bool {
        self.route != ReportSheetRoute::Idle
    }

    pub fn reasons(&self) -> &'static [ReportReason] {
        match &self.target {
            Some(target) => ReportReason::reasons(target),
            None => ReportReason::content_reasons(),
        }
    }

    pub fn offers_context(&self) -> bool {
        self.target.as_ref().is_some_and(ReportTarget::supports_context)
    }

    pub fn slots_left(&self) -> i64 {
        0_i64.max(RiseReportEvidenceUpload::maximum_attachments() - self.attachments.len() as i64)
    }

    pub fn can_submit(&self) -> bool {
        self.selected_reason.is_some()
    }

    pub fn present(
        &mut self,
        target: ReportTarget,
        subject: Option<ReportSubject>,
        context_note: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if self.route != ReportSheetRoute::Idle {
            return;
        }
        self.include_context = target.supports_context();
        self.target = Some(target);
        self.subject = subject;
        self.context_note = context_note;
        self.draft_id = Uuid::new_v4().uuid_string();
        self.selected_reason = None;
        self.description = String::new();
        self.attachments = Vec::new();
        self.route = ReportSheetRoute::Sheet;
        cx.notify();
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        match self.route {
            ReportSheetRoute::Sheet => self.route = ReportSheetRoute::ClosingSheetForDismissal,
            ReportSheetRoute::Picker => self.route = ReportSheetRoute::ClosingPickerForSheet,
            ReportSheetRoute::Idle
            | ReportSheetRoute::ClosingSheetForPicker
            | ReportSheetRoute::ClosingSheetForDismissal
            | ReportSheetRoute::ClosingPickerForSheet => return,
        }
        cx.notify();
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.forget_draft();
        self.route = ReportSheetRoute::Idle;
        cx.notify();
    }

    pub fn select(&mut self, reason: ReportReason, cx: &mut Context<Self>) {
        self.selected_reason = Some(reason);
        cx.notify();
    }

    pub fn set_description(&mut self, description: String, cx: &mut Context<Self>) {
        self.description = description;
        cx.notify();
    }

    pub fn set_include_context(&mut self, include_context: bool, cx: &mut Context<Self>) {
        self.include_context = include_context;
        cx.notify();
    }

    pub fn attach(&mut self, result: MediaPickerResult, cx: &mut Context<Self>) {
        self.append_attachments(&result, cx);
        if self.route == ReportSheetRoute::Picker {
            self.route = ReportSheetRoute::ClosingPickerForSheet;
        }
        cx.notify();
    }

    pub fn attach_dropped_files(&mut self, paths: &[std::path::PathBuf], cx: &mut Context<Self>) {
        if self.route != ReportSheetRoute::Sheet || self.slots_left() <= 0 {
            return;
        }
        let items: Vec<MediaPickerItem> = paths
            .iter()
            .filter(|path| path.is_file())
            .filter_map(|path| {
                let url = Url::from_file_path(path)?;
                let content_type = path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .and_then(UTType::from_filename_extension);
                let kind = match content_type {
                    Some(content_type) if content_type.conforms_to(&UTType::image()) => MediaPickerItemKind::Image,
                    Some(content_type) if content_type.conforms_to(&UTType::movie()) => MediaPickerItemKind::Video,
                    _ => MediaPickerItemKind::File,
                };
                let file_name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
                Some(MediaPickerItem::new(
                    Uuid::new_v4().uuid_string(),
                    kind,
                    url,
                    file_name,
                    None,
                    None,
                    None,
                    None,
                    kind == MediaPickerItemKind::File,
                ))
            })
            .collect();
        if items.is_empty() {
            return;
        }
        let draft = self.draft_id.clone();
        cx.spawn(async move |this, cx| {
            let items = DroppedVideoProbe::probing_durations(items).await;
            let _ = this.update(cx, |store, cx| {
                if store.route != ReportSheetRoute::Sheet || store.draft_id != draft {
                    return;
                }
                store.append_attachments(&MediaPickerResult::new(items, false), cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn append_attachments(&mut self, result: &MediaPickerResult, cx: &mut Context<Self>) {
        if result.was_cancelled {
            return;
        }
        let maximum_attachments = RiseReportEvidenceUpload::maximum_attachments();
        let maximum_video_duration_ms = RiseReportEvidenceUpload::maximum_video_duration_ms();
        let mut rejected_video = false;
        for item in &result.items {
            if self.attachments.len() as i64 >= maximum_attachments {
                break;
            }
            let Some(attachment) = BugReportAttachmentFactory::make(item) else {
                continue;
            };
            if attachment.kind == BugReportAttachmentKind::Video
                && let Some(duration) = attachment.duration_ms
                && duration > maximum_video_duration_ms
            {
                rejected_video = true;
                continue;
            }
            self.attachments.push(attachment);
        }
        if rejected_video {
            Notifier::send(
                tr("bug_report_video_too_long"),
                NotifierSendOptions {
                    r#type: NotifierType::Error,
                    position: NotifierPosition::Bottom,
                    ..NotifierSendOptions::default()
                },
                cx,
            );
        }
    }

    pub fn remove_attachment(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(index) = self.attachments.iter().position(|attachment| attachment.id == id) else {
            return;
        };
        let removed = self.attachments.remove(index);
        let urls: Vec<Url> = [removed.url, removed.preview_url].into_iter().flatten().collect();
        BugReportAttachmentFactory::remove_temporary_files(&urls);
        cx.notify();
    }

    pub fn open_picker(&mut self, cx: &mut Context<Self>) {
        if self.route != ReportSheetRoute::Sheet || self.slots_left() <= 0 {
            return;
        }
        self.route = ReportSheetRoute::ClosingSheetForPicker;
        cx.notify();
    }

    pub fn set_sheet_presented(&mut self, presented: bool, cx: &mut Context<Self>) {
        if presented || self.route != ReportSheetRoute::Sheet {
            return;
        }
        self.route = ReportSheetRoute::ClosingSheetForDismissal;
        cx.notify();
    }

    pub fn sheet_did_dismiss(&mut self, cx: &mut Context<Self>) {
        match self.route {
            ReportSheetRoute::ClosingSheetForPicker => self.route = ReportSheetRoute::Picker,
            ReportSheetRoute::ClosingSheetForDismissal => {
                self.forget_draft();
                self.route = ReportSheetRoute::Idle;
            }
            ReportSheetRoute::Idle
            | ReportSheetRoute::Sheet
            | ReportSheetRoute::Picker
            | ReportSheetRoute::ClosingPickerForSheet => return,
        }
        cx.notify();
    }

    pub fn sheet_presentation_refused(&mut self, cx: &mut Context<Self>) {
        self.route = ReportSheetRoute::Idle;
        cx.notify();
    }

    pub fn set_picker_presented(&mut self, presented: bool, cx: &mut Context<Self>) {
        if presented || self.route != ReportSheetRoute::Picker {
            return;
        }
        self.route = ReportSheetRoute::ClosingPickerForSheet;
        cx.notify();
    }

    pub fn picker_did_dismiss(&mut self, cx: &mut Context<Self>) {
        if self.route != ReportSheetRoute::ClosingPickerForSheet {
            return;
        }
        self.route = ReportSheetRoute::Sheet;
        cx.notify();
    }

    /// sends and closes without waiting for the reply, it continues in background and reports by toast
    /// with no connection the sheet stays with the draft
    pub fn confirm(&mut self, cx: &mut Context<Self>) {
        if self.route != ReportSheetRoute::Sheet || self.is_checking_connection || self.selected_reason.is_none() {
            return;
        }
        let draft = self.draft_id.clone();
        self.is_checking_connection = true;
        cx.notify();
        let check = ModerationInteractionsStore::shared(cx).update(cx, |store, cx| store.can_send_report_now(cx));
        cx.spawn(async move |this, cx| {
            let can_send = check.await;
            let _ = this.update(cx, |store, cx| {
                store.is_checking_connection = false;
                cx.notify();
                if !can_send || store.route != ReportSheetRoute::Sheet || store.draft_id != draft {
                    return;
                }
                store.hand_off(cx);
            });
        })
        .detach();
    }

    /// note first then the person words
    /// cut to the canon length so the server never refuses over the part the app added
    fn described(text: String, note: Option<&str>) -> String {
        let Some(note) = note.map(trimming_whitespaces_and_newlines).filter(|note| !note.is_empty()) else {
            return text;
        };
        let limit = usize::try_from(RiseLimits::service_value("report_description")).unwrap_or(0);
        format!("{note}\n{text}").chars().take(limit).collect()
    }

    fn hand_off(&mut self, cx: &mut Context<Self>) {
        let (Some(target), Some(reason)) = (self.target.clone(), self.selected_reason) else {
            return;
        };
        let trimmed = trimming_whitespaces_and_newlines(&self.description);
        let payload = ReportDraftPayload {
            draft_id: self.draft_id.clone(),
            target,
            reason,
            description: Self::described(
                if trimmed.is_empty() { tr(reason.title_key()) } else { trimmed.to_string() },
                self.context_note.as_deref(),
            ),
            attachments: self.attachments.clone(),
            include_context: self.include_context,
        };
        self.handed_off_draft_id = Some(self.draft_id.clone());
        self.dismiss(cx);
        ModerationInteractionsStore::shared(cx).update(cx, |store, cx| store.submit_report(payload, cx));
    }

    fn forget_draft(&mut self) {
        if self.handed_off_draft_id.as_deref() == Some(self.draft_id.as_str()) {
            self.handed_off_draft_id = None;
        } else {
            let urls: Vec<Url> = self
                .attachments
                .iter()
                .flat_map(|attachment| [attachment.url.clone(), attachment.preview_url.clone()])
                .flatten()
                .collect();
            BugReportAttachmentFactory::remove_temporary_files(&urls);
            RiseModerationSubmitter::shared().forget(&self.draft_id);
        }
        self.target = None;
        self.subject = None;
        self.context_note = None;
        self.selected_reason = None;
        self.description = String::new();
        self.attachments = Vec::new();
        self.include_context = true;
    }
}

/// mounts the report sheet and its picker one at a time
/// collapses to zero size while neither is up so it never eats a click
pub struct ReportSheetHost {
    store: Entity<ReportSheetStore>,
    theme_store: Entity<ThemeStore>,
    _store_observation: Subscription,
}

impl ReportSheetHost {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = ReportSheetStore::shared(cx);
        let store_observation = cx.observe(&store, |_, _, cx| cx.notify());
        Self {
            store,
            theme_store: ThemeStore::shared(cx),
            _store_observation: store_observation,
        }
    }

    fn picker(&self, cx: &App) -> AnyElement {
        let presented_store = self.store.clone();
        let presented_setter = self.store.clone();
        let complete_store = self.store.clone();
        let dismissed_store = self.store.clone();
        let slots_left = self.store.read(cx).slots_left();
        MediaPickerSheet::new(
            "report-sheet-evidence-picker",
            Binding::new(
                move |cx| presented_store.read(cx).is_picker_presented(),
                move |presented, _, cx| {
                    presented_setter.update(cx, |store, cx| store.set_picker_presented(presented, cx));
                },
            ),
            MediaPickerConfiguration::report_evidence_slots_left(slots_left),
            self.theme_store.clone(),
            move |result: MediaPickerResult, _: &mut Window, cx: &mut App| {
                let store = complete_store.clone();
                cx.defer(move |cx| store.update(cx, |store, cx| store.attach(result, cx)));
            },
        )
        .experimental_bottom_sheet(true)
        .on_dismissed(move |_: &mut Window, cx: &mut App| {
            dismissed_store.update(cx, |store, cx| store.picker_did_dismiss(cx));
        })
        .into_any_element()
    }
}

impl Render for ReportSheetHost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store = self.store.read(cx);
        let (mounts_sheet, mounts_picker, interactive) =
            (store.mounts_sheet(), store.mounts_picker(), store.has_interactive_surface());

        let mut root = div().absolute().top_0().left_0();
        root = if interactive { root.size_full() } else { root.size_0().overflow_hidden() };
        if mounts_sheet {
            root = root.child(div().absolute().inset_0().child(ReportSheet::new()));
        }
        if mounts_picker {
            root = root.child(div().absolute().inset_0().child(self.picker(cx)));
        }
        root
    }
}

/// own state of the sheet, used when a caller drives it with presentation_override
struct ReportSheetLocalState {
    reason: Option<ReportReason>,
    description: String,
    context: bool,
}

#[derive(IntoElement)]
pub struct ReportSheet {
    presentation_override: Option<Binding<bool>>,
    reasons_override: Option<Vec<ReportReason>>,
}

static DESCRIPTION_RULE: LazyLock<RiseFieldRule> =
    LazyLock::new(|| RiseFieldRule::new("report_create_report", "/description"));

const REASON_SELECTION_RESPONSE: f64 = 0.24;
const REASON_SELECTION_DAMPING: f64 = 0.8;

impl ReportSheet {
    pub fn new() -> Self {
        Self {
            presentation_override: None,
            reasons_override: None,
        }
    }

    pub fn presentation_override(mut self, presentation_override: Option<Binding<bool>>) -> Self {
        self.presentation_override = presentation_override;
        self
    }

    pub fn reasons_override(mut self, reasons_override: Option<Vec<ReportReason>>) -> Self {
        self.reasons_override = reasons_override;
        self
    }
}

impl Default for ReportSheet {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
struct ReportSheetParts {
    store: Entity<ReportSheetStore>,
    local: Entity<ReportSheetLocalState>,
    presentation_override: Option<Binding<bool>>,
    reasons_override: Option<Rc<[ReportReason]>>,
    title_width: Entity<Option<f32>>,
    details_presence: Entity<Presence<()>>,
    description_presence: Entity<Presence<()>>,
}

impl ReportSheetParts {
    fn is_overridden(&self) -> bool {
        self.presentation_override.is_some()
    }

    fn selected_reason(&self, cx: &App) -> Option<ReportReason> {
        if self.is_overridden() {
            self.local.read(cx).reason
        } else {
            self.store.read(cx).selected_reason
        }
    }

    /// typed text or attached evidence that a stray click must not throw away
    fn holds_draft(&self, cx: &App) -> bool {
        if self.is_overridden() {
            !trimming_whitespaces_and_newlines(&self.local.read(cx).description).is_empty()
        } else {
            let store = self.store.read(cx);
            !trimming_whitespaces_and_newlines(&store.description).is_empty() || !store.attachments.is_empty()
        }
    }

    fn close(&self, window: &mut Window, cx: &mut App) {
        if let Some(presentation_override) = &self.presentation_override {
            presentation_override.set(false, window, cx);
        } else {
            self.store.update(cx, |store, cx| store.dismiss(cx));
        }
    }

    fn content(&self, window: &mut Window, cx: &mut App) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        let selected = self.selected_reason(cx).map(|_| ());
        let now = RiseClock::monotonic();
        let spec = spring(REASON_SELECTION_RESPONSE, REASON_SELECTION_DAMPING);
        self.details_presence.update(cx, |presence, cx| presence.set_in(selected, spec, now, cx));
        self.description_presence.update(cx, |presence, cx| presence.set_in(selected, spec, now, cx));

        let mut column = div().w_full().flex().flex_col().gap(px(18.0)).pt(px(8.0)).child(self.reason_grid(&theme, window, cx));

        let description = self.description_field(cx);
        if let Some(field) = self.description_presence.read(cx).render_transitioned(now, window, |_| description) {
            column = column.child(field);
        }
        let mut details = div().w_full().flex().flex_col().gap(px(18.0)).child(self.evidence_section(&theme, cx));
        if self.is_overridden() || self.store.read(cx).offers_context() {
            details = details.child(self.context_row(&theme));
        }
        if let Some(details) = self.details_presence.read(cx).render_transitioned(now, window, |_| details) {
            column = column.child(details);
        }
        column = column.child(div().min_h(px(24.0)));

        let submit_parts = self.clone();
        let drop_parts = self.clone();
        let mut root = div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.bg._100)
            .on_drop(move |paths: &ExternalPaths, _, cx| {
                if drop_parts.is_overridden() {
                    return;
                }
                drop_parts.store.update(cx, |store, cx| store.attach_dropped_files(paths.paths(), cx));
            })
            .child(self.header());
        if !self.is_overridden()
            && let Some(subject) = self.store.read(cx).subject().cloned()
        {
            root = root.child(Self::subject_row(&subject, &theme));
        }
        let root = root.child(
            div()
                .flex_1()
                .min_h_0()
                .child(scroll_view("report-sheet-scroll").shows_indicators(false).child(column)),
        );
        SheetConfirmSurface::new("report-sheet-surface", root, move |window, cx| {
            if submit_parts.selected_reason(cx).is_none() {
                return;
            }
            if submit_parts.is_overridden() {
                submit_parts.close(window, cx);
            } else {
                submit_parts.store.update(cx, |store, cx| store.confirm(cx));
            }
        })
        .into_any_element()
    }

    fn header(&self) -> AnyElement {
        let parts = self.clone();
        SheetHeaderBar::new("report-sheet-header", tr("report_sheet_title"), move |window, cx| parts.close(window, cx))
            .right_system_image(None)
            .into_any_element()
    }

    fn subject_row(subject: &ReportSubject, theme: &AppTheme) -> AnyElement {
        let mut names = div().flex().flex_col().gap(px(2.0)).min_w_0().child(
            MainText::new(subject.name.clone())
                .px(15.0)
                .font_weight(FontWeight::SEMIBOLD)
                .color(theme.text.primary)
                .line_limit(1),
        );
        if !subject.tag.is_empty() {
            names = names.child(SecondaryText::new(format!("@{}", subject.tag)).px(13.0).line_limit(1));
        }
        div()
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.0))
            .px(px(16.0))
            .py(px(8.0))
            .child(
                UserLogoUi::new("report-sheet-subject-logo")
                    .source(SharedString::from(subject.avatar_url.clone()))
                    .name(SharedString::from(subject.name.clone()))
                    .user_id(subject.user_id.clone().map(SharedString::from))
                    .size(36.0),
            )
            .child(names)
            .child(div().flex_1())
            .into_any_element()
    }

    fn reasons(&self, cx: &App) -> Rc<[ReportReason]> {
        match &self.reasons_override {
            Some(reasons) => reasons.clone(),
            None => Rc::from(self.store.read(cx).reasons()),
        }
    }

    fn reason_grid(&self, theme: &AppTheme, window: &mut Window, cx: &mut App) -> AnyElement {
        let reasons = self.reasons(cx);
        let text_delta = ThemeStore::shared(cx).read(cx).text_delta();
        let titles: Vec<String> = reasons.iter().map(|reason| tr(reason.title_key())).collect();
        let widest_word =
            ReportReasonTitle::widest_word_width(&titles, ReportReasonTitle::base_size(text_delta), window);
        let selected = self.selected_reason(cx);

        let mut grid = div().w_full().flex().flex_col().gap(px(10.0)).px(px(16.0));
        for (row_index, row) in reasons.chunks(2).enumerate() {
            let mut line = div().w_full().flex().flex_row().items_stretch().gap(px(10.0));
            for (column_index, reason) in row.iter().enumerate() {
                let title = titles[row_index * 2 + column_index].clone();
                line = line.child(div().flex_1().min_w_0().flex().flex_col().child(self.reason_tile(
                    *reason,
                    title,
                    selected == Some(*reason),
                    widest_word,
                    text_delta,
                    theme,
                    window,
                    cx,
                )));
            }
            if row.len() < 2 {
                line = line.child(div().flex_1());
            }
            grid = grid.child(line);
        }

        div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(div().px(px(16.0)).child(SecondaryText::new(tr("report_sheet_pick_reason")).px(13.0)))
            .child(grid)
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn reason_tile(
        &self,
        reason: ReportReason,
        title: String,
        selected: bool,
        widest_word: f32,
        text_delta: f32,
        theme: &AppTheme,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let tint = ModerationTint::reason(reason);
        let now = RiseClock::monotonic();
        let spec = spring(REASON_SELECTION_RESPONSE, REASON_SELECTION_DAMPING);
        let fill_target = if selected { tint.opacity(0.16) } else { theme.bg._200 };
        let border_target = if selected { tint } else { transparent_black() };
        let fill_state = window.use_keyed_state(
            (ElementId::from("report-reason-fill"), reason.id()),
            cx,
            |_, _| Animated::new(fill_target),
        );
        let border_state = window.use_keyed_state(
            (ElementId::from("report-reason-border"), reason.id()),
            cx,
            |_, _| Animated::new(border_target),
        );
        let fill = fill_state.update(cx, |animated, cx| {
            animated.set_in(fill_target, spec, now, cx);
            animated.frame_value(now, window)
        });
        let border = border_state.update(cx, |animated, cx| {
            animated.set_in(border_target, spec, now, cx);
            animated.frame_value(now, window)
        });

        let parts = self.clone();
        plain((ElementId::from("report-reason"), reason.id()))
            .on_press(move |_, cx| {
                let _ = impact(ImpactStyle::Light);
                if parts.is_overridden() {
                    parts.local.update(cx, |local, cx| {
                        local.reason = Some(reason);
                        cx.notify();
                    });
                } else {
                    parts.store.update(cx, |store, cx| store.select(reason, cx));
                }
            })
            .w_full()
            .flex_1()
            .min_h(px(56.0))
            .p(px(10.0))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.0))
            .bg(fill)
            .rounded(px(14.0))
            .border(px(1.5))
            .border_color(border)
            .overflow_hidden()
            .child(
                div()
                    .size(px(32.0))
                    .flex_none()
                    .rounded(px(9.0))
                    .bg(tint)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(symbol(reason.icon_name(), 14.0, white())),
            )
            .child(
                div().flex_1().min_w_0().child(ReportReasonTitle {
                    id: (ElementId::from("report-reason-title"), reason.id()).into(),
                    text: title.into(),
                    color: theme.text.primary,
                    widest_word,
                    text_delta,
                    available_width: self.title_width.clone(),
                }),
            )
            .into_any_element()
    }

    fn description_field(&self, cx: &App) -> Div {
        let value = if self.is_overridden() {
            self.local.read(cx).description.clone()
        } else {
            self.store.read(cx).description.clone()
        };
        let parts = self.clone();
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .px(px(16.0))
            .child(SecondaryText::new(tr("report_description_label")).px(13.0))
            .child(
                TextAreaUi::new("report-sheet-description")
                    .values(HashMap::from([("description".to_string(), value)]))
                    .name("description")
                    .placeholder(tr("report_description_placeholder"))
                    .min_lines(3)
                    .max_lines(8)
                    .rule(*DESCRIPTION_RULE)
                    .set_value(move |_, value, _, cx| {
                        let value = value.to_string();
                        if parts.is_overridden() {
                            parts.local.update(cx, |local, cx| {
                                local.description = value;
                                cx.notify();
                            });
                        } else {
                            parts.store.update(cx, |store, cx| store.set_description(value, cx));
                        }
                    }),
            )
    }

    fn evidence_section(&self, theme: &AppTheme, cx: &App) -> AnyElement {
        let attachments: Vec<BugReportAttachment> =
            if self.is_overridden() { Vec::new() } else { self.store.read(cx).attachments().to_vec() };
        let maximum = RiseReportEvidenceUpload::maximum_attachments();

        let mut row = div().w_full().flex().flex_row().flex_wrap().gap(px(8.0)).px(px(16.0));
        for attachment in &attachments {
            let store = self.store.clone();
            let id = attachment.id.clone();
            row = row.child(ReportEvidenceThumbnail::new(attachment.clone(), false, move |_, cx| {
                store.update(cx, |store, cx| store.remove_attachment(&id, cx));
            }));
        }
        row = row.child(self.add_evidence_button(theme, cx));

        div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_row()
                    .items_center()
                    .px(px(16.0))
                    .child(SecondaryText::new(tr("report_evidence_section")).px(13.0))
                    .child(div().flex_1().min_w(px(8.0)))
                    .child(SecondaryText::new(format!("{}/{}", attachments.len(), maximum)).px(13.0)),
            )
            .child(row)
            .child(div().px(px(16.0)).child(SecondaryText::new(tr("report_evidence_hint")).px(12.0)))
            .into_any_element()
    }

    fn add_evidence_button(&self, theme: &AppTheme, cx: &App) -> AnyElement {
        let disabled = !self.is_overridden() && self.store.read(cx).slots_left() == 0;
        let parts = self.clone();
        let secondary = theme.text.secondary;
        plain("report-evidence-add")
            .disabled(disabled)
            .on_press(move |_, cx| {
                if parts.is_overridden() {
                    return;
                }
                parts.store.update(cx, |store, cx| store.open_picker(cx));
            })
            .size(px(72.0))
            .flex_none()
            .rounded(px(10.0))
            .border(px(1.0))
            .border_color(secondary.opacity(0.35))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(4.0))
            .text_color(secondary)
            .opacity(if disabled { rise_ui::swiftui::button_style::DISABLED_OPACITY } else { 1.0 })
            .child(symbol("plus", 18.0, secondary))
            .child(div().text_size(px(11.0)).child(tr("report_evidence_add")))
            .into_any_element()
    }

    fn context_row(&self, theme: &AppTheme) -> AnyElement {
        let value = if self.is_overridden() {
            Binding::entity(
                self.local.clone(),
                |local: &ReportSheetLocalState| local.context,
                |local: &mut ReportSheetLocalState, value: bool, cx: &mut Context<ReportSheetLocalState>| {
                    local.context = value;
                    cx.notify();
                },
            )
        } else {
            Binding::entity(
                self.store.clone(),
                |store: &ReportSheetStore| store.include_context,
                |store: &mut ReportSheetStore, value: bool, cx: &mut Context<ReportSheetStore>| {
                    store.set_include_context(value, cx)
                },
            )
        };
        div()
            .mx(px(16.0))
            .p(px(12.0))
            .flex()
            .flex_row()
            .items_start()
            .gap(px(12.0))
            .bg(theme.bg._200)
            .rounded(px(14.0))
            .child(
                div()
                    .size(px(32.0))
                    .flex_none()
                    .rounded(px(9.0))
                    .bg(theme.primary._100.opacity(0.14))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(symbol("text.bubble.fill", 14.0, theme.primary._100)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.0))
                    .min_w_0()
                    .child(MainText::new(tr("report_context_toggle")).px(14.0).font_weight(FontWeight::SEMIBOLD))
                    .child(SecondaryText::new(tr("report_context_hint")).px(12.0)),
            )
            .child(div().flex_1().min_w(px(8.0)))
            .child(SwitchUi::new("report-sheet-context", value))
            .into_any_element()
    }

    fn footer(&self, cx: &App) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        let parts = self.clone();
        div()
            .w_full()
            .px(px(16.0))
            .py(px(10.0))
            .child(
                ButtonUi::new("report-sheet-submit")
                    .disabled(self.selected_reason(cx).is_none())
                    .is_pending(self.store.read(cx).is_checking_connection())
                    .background_color(theme.primary._100)
                    .height(50.0)
                    .loader_color(theme.content_on(theme.primary._100))
                    .on_press(move |window, cx| {
                        if parts.is_overridden() {
                            parts.close(window, cx);
                        } else {
                            parts.store.update(cx, |store, cx| store.confirm(cx));
                        }
                    })
                    .child(
                        MainText::new(tr("report_submit"))
                            .px(16.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(white()),
                    ),
            )
            .into_any_element()
    }
}

impl RenderOnce for ReportSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let store = ReportSheetStore::shared(cx);
        let local = window.use_keyed_state("report-sheet-local", cx, |_, _| ReportSheetLocalState {
            reason: None,
            description: String::new(),
            context: true,
        });
        let initial = self.presentation_override.is_none() && store.read(cx).selected_reason.is_some()
            || self.presentation_override.is_some() && local.read(cx).reason.is_some();
        let initial = initial.then_some(());
        let parts = ReportSheetParts {
            store: store.clone(),
            local,
            presentation_override: self.presentation_override.clone(),
            reasons_override: self.reasons_override.map(Rc::from),
            title_width: window.use_keyed_state("report-sheet-title-width", cx, |_, _| None),
            details_presence: window.use_keyed_state("report-sheet-details", cx, |_, _| {
                Presence::new(initial).with_transition(Transition::opacity())
            }),
            description_presence: window.use_keyed_state("report-sheet-description-presence", cx, |_, _| {
                Presence::new(initial).with_transition(Transition::opacity().combined(Transition::move_edge(Edge::Top)))
            }),
        };

        let presented = match &self.presentation_override {
            Some(presentation_override) => presentation_override.clone(),
            None => {
                let reader = store.clone();
                let writer = store.clone();
                Binding::new(
                    move |cx| reader.read(cx).is_sheet_presented(),
                    move |presented, _, cx| writer.update(cx, |store, cx| store.set_sheet_presented(presented, cx)),
                )
            }
        };

        let holds_draft = parts.holds_draft(cx);
        let content_parts = parts.clone();
        let footer_parts = parts.clone();
        let did_dismiss_parts = parts.clone();
        let refused_parts = parts;
        BottomSheetUi::new(
            "report-sheet",
            presented,
            move |window, cx| content_parts.content(window, cx),
            move |_, cx| footer_parts.footer(cx),
        )
        .snap_fractions(vec![0.7, 0.94])
        .dismisses_keyboard_before_backdrop_dismiss(true)
        .dismisses_on_backdrop_tap(!holds_draft)
        .footer_shows_top_border(true)
        .content_scrollable(true)
        .sheet_background(Some(theme.bg._100))
        .footer_background(Some(theme.bg._100))
        .on_did_dismiss(move |_, cx| {
            if !did_dismiss_parts.is_overridden() {
                did_dismiss_parts.store.update(cx, |store, cx| store.sheet_did_dismiss(cx));
            }
        })
        .on_presentation_refused(move |window, cx| {
            if refused_parts.is_overridden() {
                refused_parts.close(window, cx);
            } else {
                refused_parts.store.update(cx, |store, cx| store.sheet_presentation_refused(cx));
            }
        })
    }
}

/// system symbol at a point size, an unmapped symbol keeps its box empty
fn symbol(name: &str, side: f32, color: Hsla) -> AnyElement {
    match IconUi::sized(name, px(side), color, name.ends_with(".fill")) {
        Some(icon) => icon.into_any_element(),
        None => div().size(px(side)).into_any_element(),
    }
}

/// a two column tile is too narrow for long words like harassment in long locales
/// text breaks them mid word so this steps size down until widest_word fits on one line
#[derive(IntoElement)]
struct ReportReasonTitle {
    id: ElementId,
    text: SharedString,
    color: Hsla,
    widest_word: f32,
    text_delta: f32,
    available_width: Entity<Option<f32>>,
}

impl ReportReasonTitle {
    const SCALES: [f32; 5] = [1.0, 0.92, 0.85, 0.78, 0.7];

    fn base_size(text_delta: f32) -> f32 {
        1.0_f32.max(13.0 + text_delta)
    }

    fn fitting_scale(widest_word: f32, available_width: Option<f32>) -> f32 {
        let Some(available_width) = available_width else {
            return Self::SCALES[0];
        };
        Self::SCALES
            .iter()
            .copied()
            .find(|scale| (widest_word * scale).ceil() + 1.0 <= available_width)
            .unwrap_or(Self::SCALES[Self::SCALES.len() - 1])
    }

    fn widest_word_width(texts: &[String], size: f32, window: &Window) -> f32 {
        let face = gpui::Font {
            weight: FontWeight::SEMIBOLD,
            ..gpui::font(FONT_FAMILY)
        };
        let mut widest = 0.0_f32;
        for text in texts {
            for word in text.split(|c: char| !(c.is_alphanumeric() || c == '\'')).filter(|word| !word.is_empty()) {
                let runs = [gpui::TextRun {
                    len: word.len(),
                    font: face.clone(),
                    color: black(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }];
                let width = window
                    .text_system()
                    .shape_line(SharedString::from(word.to_string()), px(size), &runs, None)
                    .width();
                widest = widest.max(f32::from(width));
            }
        }
        widest
    }
}

impl RenderOnce for ReportReasonTitle {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let base = Self::base_size(self.text_delta);
        let scale = Self::fitting_scale(self.widest_word, *self.available_width.read(cx));
        let available_width = self.available_width.clone();
        div()
            .relative()
            .w_full()
            .text_size(px(base * scale))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(self.color)
            .child(MinimumScaleText::new(self.text, 0.85).line_limit(2))
            .child(measure(self.id, move |bounds, _, cx| {
                let width = f32::from(bounds.size.width);
                available_width.update(cx, |available, cx| {
                    if *available != Some(width) {
                        *available = Some(width);
                        cx.notify();
                    }
                });
            }))
    }
}

type RemoveAction = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct ReportEvidenceThumbnail {
    attachment: BugReportAttachment,
    disabled: bool,
    on_remove: RemoveAction,
}

impl ReportEvidenceThumbnail {
    pub fn new(
        attachment: BugReportAttachment,
        disabled: bool,
        on_remove: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            attachment,
            disabled,
            on_remove: Rc::new(on_remove),
        }
    }
}

impl RenderOnce for ReportEvidenceThumbnail {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let mut image = div().relative().size(px(72.0)).rounded(px(10.0)).overflow_hidden();
        image = match &self.attachment.preview_url {
            Some(preview) => image.child(
                CleverImageUi::new(SharedString::from(preview.absolute_string().to_string()))
                    .id((ElementId::from("report-evidence-image"), self.attachment.id.clone()))
                    .content_mode(ContentMode::Fill)
                    .corner_radius(10.0)
                    .shows_progress_indicator(false)
                    .maximum_decode_pixel_dimension(256_i64)
                    .shows_failure_icon(false),
            ),
            None => image.child(div().absolute().inset_0().rounded(px(10.0)).bg(theme.bg._200)),
        };
        if self.attachment.kind == BugReportAttachmentKind::Video {
            image = image.child(
                div()
                    .absolute()
                    .inset_0()
                    .rounded(px(10.0))
                    .bg(black().opacity(0.28))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(symbol("play.fill", 16.0, white())),
            );
        }

        let on_remove = self.on_remove.clone();
        div().relative().size(px(72.0)).flex_none().child(image).child(
            div().absolute().top_0().right_0().p(px(4.0)).child(
                plain((ElementId::from("report-evidence-remove"), self.attachment.id.clone()))
                    .disabled(self.disabled)
                    .on_press(move |window, cx| on_remove(window, cx))
                    .rounded_full()
                    .bg(theme.bg._100)
                    .child(symbol("xmark.circle.fill", 18.0, theme.text.primary)),
            ),
        )
    }
}
