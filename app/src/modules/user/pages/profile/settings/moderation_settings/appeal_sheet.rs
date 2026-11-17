use std::collections::HashMap;

use gpui::{App, Context, Entity, ExternalPaths, FontWeight, Hsla, Render, Subscription, Window};
use rise_core::foundation::foundation_text::character_count;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_platform::uniform_type::UTType;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::keyboard_dismiss::SheetConfirmSurface;
use rise_ui::IconUi;
use rise_ui::button_ui::ButtonUi;
use rise_ui::main_text::MainText;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::sheet_header_bar::SheetHeaderBar;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::text_area_ui::TextAreaUi;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::core::widgets::sheets::report_sheet::ReportEvidenceThumbnail;
use crate::modules::diagnostics::engine::core::rise_bug_report_upload::{BugReportAttachment, BugReportAttachmentKind};
use crate::modules::diagnostics::stores::diagnostics::diagnostics_interactions::bug_report_interactions::BugReportAttachmentFactory;
use crate::modules::media_picker::api::media_picker_configuration::MediaPickerConfiguration;
use crate::modules::media_picker::api::media_picker_result::{MediaPickerItem, MediaPickerItemKind, MediaPickerResult};
use crate::modules::media_picker::export::dropped_video_probe::DroppedVideoProbe;
use crate::modules::media_picker::ui::media_picker_sheet::MediaPickerSheet;
use crate::modules::moderation::components::moderation_type_tints::UserBlockTypeTintExt;
use crate::modules::moderation::engine::core::rise_report_evidence_upload::RiseReportEvidenceUpload;
use crate::modules::moderation::engine::rise_moderation_submitter::{AppealDraftPayload, RiseModerationSubmitter};
use crate::modules::moderation::engine::rise_moderation_wire::{SanctionHistoryItemDTO, UserBlockDTO};
use crate::modules::moderation::stores::moderation_actions::moderation_types::{
    ModerationAuditActionExt, ReportReasonExt, UserBlockTypeExt,
};
use crate::modules::moderation::stores::moderation_interactions::moderation_interactions::ModerationInteractionsStore;
use crate::modules::user::shared::profile_field_rules::ModerationFieldRules;


#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AppealSheetRoute {
    #[default]
    Idle,
    Sheet,
    ClosingSheetForPicker,
    ClosingSheetForDismissal,
    Picker,
    ClosingPickerForSheet,
}

/// appeal a sanctioned person writes with proof attached. sheet and picker take turns since
/// two modals dont stack
pub struct AppealSheetStore {
    route: AppealSheetRoute,
    block: Option<UserBlockDTO>,
    sanction: Option<SanctionHistoryItemDTO>,
    draft_id: String,
    pub text: String,
    attachments: Vec<BugReportAttachment>,
    is_submitting: bool,
}

impl AppealSheetStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            route: AppealSheetRoute::Idle,
            block: None,
            sanction: None,
            draft_id: Uuid::new_v4().uuid_string(),
            text: String::new(),
            attachments: Vec::new(),
            is_submitting: false,
        }
    }

    pub fn route(&self) -> AppealSheetRoute {
        self.route
    }

    pub fn block(&self) -> Option<&UserBlockDTO> {
        self.block.as_ref()
    }

    pub fn sanction(&self) -> Option<&SanctionHistoryItemDTO> {
        self.sanction.as_ref()
    }

    pub fn draft_id(&self) -> &str {
        &self.draft_id
    }

    pub fn attachments(&self) -> &[BugReportAttachment] {
        &self.attachments
    }

    pub fn is_submitting(&self) -> bool {
        self.is_submitting
    }

    pub fn mounts_sheet(&self) -> bool {
        matches!(
            self.route,
            AppealSheetRoute::Sheet | AppealSheetRoute::ClosingSheetForPicker | AppealSheetRoute::ClosingSheetForDismissal
        )
    }

    pub fn is_sheet_presented(&self) -> bool {
        self.route == AppealSheetRoute::Sheet
    }

    pub fn mounts_picker(&self) -> bool {
        matches!(self.route, AppealSheetRoute::Picker | AppealSheetRoute::ClosingPickerForSheet)
    }

    pub fn is_picker_presented(&self) -> bool {
        self.route == AppealSheetRoute::Picker
    }

    pub fn has_interactive_surface(&self) -> bool {
        self.route != AppealSheetRoute::Idle
    }

    pub fn slots_left(&self) -> i64 {
        (RiseReportEvidenceUpload::maximum_attachments() - self.attachments.len() as i64).max(0)
    }

    pub fn trimmed_text(&self) -> String {
        ModerationFieldRules::appeal_text()
            .schema
            .as_ref()
            .map(|schema| schema.normalize(&self.text))
            .unwrap_or_else(|| self.text.clone())
    }

    pub fn is_valid(&self) -> bool {
        ModerationFieldRules::appeal_text().is_satisfied(&self.text)
    }

    pub fn present_block(&mut self, block: UserBlockDTO, cx: &mut Context<Self>) {
        if self.route != AppealSheetRoute::Idle || !block.allows_appeal() {
            return;
        }
        self.block = Some(block);
        self.sanction = None;
        self.draft_id = Uuid::new_v4().uuid_string();
        self.text = String::new();
        self.attachments = Vec::new();
        self.route = AppealSheetRoute::Sheet;
        cx.notify();
    }

    pub fn present_sanction(&mut self, sanction: SanctionHistoryItemDTO, cx: &mut Context<Self>) {
        if self.route != AppealSheetRoute::Idle || !sanction.allows_appeal() {
            return;
        }
        self.block = None;
        self.sanction = Some(sanction);
        self.draft_id = Uuid::new_v4().uuid_string();
        self.text = String::new();
        self.attachments = Vec::new();
        self.route = AppealSheetRoute::Sheet;
        cx.notify();
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        if self.is_submitting {
            return;
        }
        match self.route {
            AppealSheetRoute::Sheet => self.route = AppealSheetRoute::ClosingSheetForDismissal,
            AppealSheetRoute::Picker => self.route = AppealSheetRoute::ClosingPickerForSheet,
            _ => {}
        }
        cx.notify();
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.forget_draft();
        self.route = AppealSheetRoute::Idle;
        cx.notify();
    }

    pub fn update_text(&mut self, value: &str, cx: &mut Context<Self>) {
        if self.is_submitting {
            return;
        }
        self.text = ModerationFieldRules::appeal_text().clamped(value);
        cx.notify();
    }

    pub fn attach(&mut self, result: MediaPickerResult, cx: &mut Context<Self>) {
        if !self.is_submitting && !result.was_cancelled {
            self.append_attachments(&result, cx);
        }
        if self.route == AppealSheetRoute::Picker {
            self.route = AppealSheetRoute::ClosingPickerForSheet;
        }
        cx.notify();
    }

    pub fn attach_dropped_files(&mut self, paths: &[std::path::PathBuf], cx: &mut Context<Self>) {
        if self.route != AppealSheetRoute::Sheet || self.is_submitting || self.slots_left() <= 0 {
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
                if store.route != AppealSheetRoute::Sheet || store.is_submitting || store.draft_id != draft {
                    return;
                }
                store.append_attachments(&MediaPickerResult::new(items, false), cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn append_attachments(&mut self, result: &MediaPickerResult, cx: &mut Context<Self>) {
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
        if self.is_submitting {
            return;
        }
        let Some(index) = self.attachments.iter().position(|attachment| attachment.id == id) else {
            return;
        };
        let removed = self.attachments.remove(index);
        let urls: Vec<Url> = [removed.url, removed.preview_url].into_iter().flatten().collect();
        BugReportAttachmentFactory::remove_temporary_files(&urls);
        cx.notify();
    }

    pub fn open_picker(&mut self, cx: &mut Context<Self>) {
        if self.route != AppealSheetRoute::Sheet || self.is_submitting || self.slots_left() <= 0 {
            return;
        }
        self.route = AppealSheetRoute::ClosingSheetForPicker;
        cx.notify();
    }

    pub fn set_sheet_presented(&mut self, presented: bool, cx: &mut Context<Self>) {
        if presented || self.route != AppealSheetRoute::Sheet || self.is_submitting {
            return;
        }
        self.route = AppealSheetRoute::ClosingSheetForDismissal;
        cx.notify();
    }

    pub fn sheet_did_dismiss(&mut self, cx: &mut Context<Self>) {
        match self.route {
            AppealSheetRoute::ClosingSheetForPicker => self.route = AppealSheetRoute::Picker,
            AppealSheetRoute::ClosingSheetForDismissal => {
                self.forget_draft();
                self.route = AppealSheetRoute::Idle;
            }
            _ => {}
        }
        cx.notify();
    }

    pub fn sheet_presentation_refused(&mut self, cx: &mut Context<Self>) {
        self.route = AppealSheetRoute::Idle;
        cx.notify();
    }

    pub fn set_picker_presented(&mut self, presented: bool, cx: &mut Context<Self>) {
        if presented || self.route != AppealSheetRoute::Picker {
            return;
        }
        self.route = AppealSheetRoute::ClosingPickerForSheet;
        cx.notify();
    }

    pub fn picker_did_dismiss(&mut self, cx: &mut Context<Self>) {
        if self.route != AppealSheetRoute::ClosingPickerForSheet {
            return;
        }
        self.route = AppealSheetRoute::Sheet;
        cx.notify();
    }

    pub fn submit(&mut self, cx: &mut Context<Self>) {
        if self.route != AppealSheetRoute::Sheet || self.is_submitting || !self.is_valid() {
            return;
        }
        let payload = if let Some(block) = &self.block {
            AppealDraftPayload {
                draft_id: self.draft_id.clone(),
                block_id: block.id(),
                report_id: block.report_id,
                block_type: Some(block.block_type.clone()),
                text: self.trimmed_text(),
                attachments: self.attachments.clone(),
            }
        } else if let Some(report_id) = self.sanction.as_ref().and_then(|sanction| sanction.report_id) {
            AppealDraftPayload {
                draft_id: self.draft_id.clone(),
                block_id: String::new(),
                report_id: Some(report_id),
                block_type: None,
                text: self.trimmed_text(),
                attachments: self.attachments.clone(),
            }
        } else {
            return;
        };
        let for_sanction = self.block.is_none();
        let draft_id = payload.draft_id.clone();
        let block_id = payload.block_id.clone();
        self.is_submitting = true;
        cx.notify();
        let this = cx.weak_entity();
        ModerationInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.submit_appeal(
                payload,
                move |completed, cx: &mut App| {
                    let _ = this.update(cx, |this, cx| {
                        let current_block_id = this.block.as_ref().map(|block| block.id()).unwrap_or_default();
                        if this.draft_id != draft_id || current_block_id != block_id || this.route != AppealSheetRoute::Sheet {
                            return;
                        }
                        this.is_submitting = false;
                        cx.notify();
                        if completed {
                            this.dismiss(cx);
                            if for_sanction {
                                ModerationInteractionsStore::shared(cx).update(cx, |store, cx| store.refresh_blocks(cx));
                            }
                        }
                    });
                },
                cx,
            );
        });
    }

    fn forget_draft(&mut self) {
        let urls: Vec<Url> = self
            .attachments
            .iter()
            .flat_map(|attachment| [attachment.url.clone(), attachment.preview_url.clone()])
            .flatten()
            .collect();
        BugReportAttachmentFactory::remove_temporary_files(&urls);
        RiseModerationSubmitter::shared().forget(&self.draft_id);
        self.draft_id = Uuid::new_v4().uuid_string();
        self.is_submitting = false;
        self.block = None;
        self.sanction = None;
        self.text = String::new();
        self.attachments = Vec::new();
    }
}

/// mounts the appeal sheet and its picker one at a time,
/// zero size while neither is up so it never eats a click
pub struct AppealSheetHost {
    store: Entity<AppealSheetStore>,
    theme_store: Entity<ThemeStore>,
    _store_observation: Subscription,
}

impl AppealSheetHost {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = AppealSheetStore::shared(cx);
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
            "appeal-sheet-evidence-picker",
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

impl Render for AppealSheetHost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store = self.store.read(cx);
        let (mounts_sheet, mounts_picker, interactive) =
            (store.mounts_sheet(), store.mounts_picker(), store.has_interactive_surface());

        let mut root = div().absolute().top_0().left_0();
        root = if interactive { root.size_full() } else { root.size_0().overflow_hidden() };
        if mounts_sheet {
            root = root.child(div().absolute().inset_0().child(AppealSheet::new()));
        }
        if mounts_picker {
            root = root.child(div().absolute().inset_0().child(self.picker(cx)));
        }
        root
    }
}

#[derive(IntoElement)]
pub struct AppealSheet;

impl AppealSheet {
    pub fn new() -> Self {
        Self
    }

    fn max_chars() -> usize {
        ModerationFieldRules::appeal_text().maximum().unwrap_or(0)
    }

    fn content(store: &Entity<AppealSheetStore>, theme: &AppTheme, cx: &App) -> AnyElement {
        let mut column = vstack(18.0).align(HorizontalAlignment::Leading).w_full().pt(px(8.0));
        let store_value = store.read(cx);
        if let Some(block) = store_value.block() {
            column = column.child(Self::sanction_card(block, theme));
        } else if let Some(sanction) = store_value.sanction() {
            column = column.child(Self::decision_card(sanction, theme));
        }
        column = column
            .child(Self::text_field(store, cx))
            .child(Self::evidence_section(store, theme, cx))
            .child(div().min_h(px(24.0)));

        let header_store = store.clone();
        let drop_store = store.clone();
        let submit_store = store.clone();
        let root = div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.bg._100)
            .on_drop(move |paths: &ExternalPaths, _, cx| {
                drop_store.update(cx, |store, cx| store.attach_dropped_files(paths.paths(), cx));
            })
            .child(
                SheetHeaderBar::new("appeal-sheet-header", tr("appeal_sheet_title"), move |_, cx| {
                    header_store.update(cx, |store, cx| store.dismiss(cx));
                })
                .right_system_image(None),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(scroll_view("appeal-sheet-scroll").shows_indicators(false).child(column)),
            );
        SheetConfirmSurface::new("appeal-sheet-surface", root, move |_, cx| {
            submit_store.update(cx, |store, cx| store.submit(cx));
        })
        .into_any_element()
    }

    fn card_title(icon: &str, accent: Hsla, title: String) -> AnyElement {
        hstack(10.0)
            .w_full()
            .child(
                div()
                    .size(px(40.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(accent.opacity(0.16))
                    .clip_shape(Shape::continuous(10.0))
                    .child(symbol(icon, 18.0, accent)),
            )
            .child(
                vstack(2.0)
                    .align(HorizontalAlignment::Leading)
                    .flex_1()
                    .min_w_0()
                    .child(MainText::new(title).px(15.0).font_weight(FontWeight::BOLD))
                    .child(div().w_full().min_w_0().child(SecondaryText::new(tr("appeal_sheet_subtitle")).px(12.0))),
            )
            .into_any_element()
    }

    fn why_box(why: String, theme: &AppTheme) -> AnyElement {
        vstack(4.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .p(px(10.0))
            .bg(theme.bg._100)
            .clip_shape(Shape::continuous(10.0))
            .child(SecondaryText::new(tr("blocks_why_label")).px(12.0))
            .child(div().w_full().child(MainText::new(why).px(14.0)))
            .into_any_element()
    }

    fn card(title: AnyElement, why: Option<AnyElement>, theme: &AppTheme) -> AnyElement {
        div()
            .w_full()
            .px(px(16.0))
            .child(
                vstack(10.0)
                    .align(HorizontalAlignment::Leading)
                    .w_full()
                    .p(px(12.0))
                    .bg(theme.bg._200)
                    .clip_shape(Shape::continuous(14.0))
                    .child(title)
                    .children(why),
            )
            .into_any_element()
    }

    fn sanction_card(block: &UserBlockDTO, theme: &AppTheme) -> AnyElement {
        let block_type = block.resolved_type();
        let accent = block_type.accent_color(theme);
        let title = Self::card_title(block_type.icon_name(), accent, tr(block_type.title_key()));
        let why = Self::why_text(block).map(|why| Self::why_box(why, theme));
        Self::card(title, why, theme)
    }

    fn decision_card(sanction: &SanctionHistoryItemDTO, theme: &AppTheme) -> AnyElement {
        let accent = theme.semantic.error_100;
        let action = sanction.resolved_action();
        let title = Self::card_title(
            action.map(|action| action.icon_name()).unwrap_or("shield"),
            accent,
            action.map(|action| tr(action.title_key())).unwrap_or_else(|| sanction.action.clone()),
        );
        let why = sanction.resolved_report_reason().map(|reason| Self::why_box(tr(reason.title_key()), theme));
        Self::card(title, why, theme)
    }

    fn why_text(block: &UserBlockDTO) -> Option<String> {
        if let Some(reason) = block.resolved_report_reason() {
            return Some(tr(reason.title_key()));
        }
        block.reason.as_ref().filter(|reason| !reason.is_empty()).cloned()
    }

    fn text_field(store: &Entity<AppealSheetStore>, cx: &App) -> AnyElement {
        let store_value = store.read(cx);
        let is_submitting = store_value.is_submitting();
        let setter_store = store.clone();
        vstack(6.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .px(px(16.0))
            .child(SecondaryText::new(tr("appeal_text_label")).px(13.0))
            .child(
                div()
                    .relative()
                    .w_full()
                    .child(
                        TextAreaUi::new("appeal-sheet-text")
                            .values(HashMap::from([("text".to_string(), store_value.text.clone())]))
                            .name("text")
                            .placeholder(tr("appeal_text_placeholder"))
                            .min_lines(4)
                            .max_lines(10)
                            .rule(ModerationFieldRules::appeal_text())
                            .set_value(move |_, value, _, cx| {
                                setter_store.update(cx, |store, cx| store.update_text(value, cx));
                            }),
                    )
                    .when(is_submitting, |field| field.child(div().absolute().inset_0().occlude())),
            )
            .child(
                hstack(None).w_full().child(spacer()).child(
                    SecondaryText::new(format!("{}/{}", character_count(&store_value.trimmed_text()), Self::max_chars())).px(11.0),
                ),
            )
            .into_any_element()
    }

    fn evidence_section(store: &Entity<AppealSheetStore>, theme: &AppTheme, cx: &App) -> AnyElement {
        let store_value = store.read(cx);
        let is_submitting = store_value.is_submitting();
        let slots_left = store_value.slots_left();
        let attachments = store_value.attachments().to_vec();
        let maximum = RiseReportEvidenceUpload::maximum_attachments();
        let secondary = theme.text.secondary;

        let mut row = hstack(8.0).flex_none().px(px(16.0));
        for attachment in &attachments {
            let remove_store = store.clone();
            let id = attachment.id.clone();
            row = row.child(ReportEvidenceThumbnail::new(attachment.clone(), is_submitting, move |_, cx| {
                remove_store.update(cx, |store, cx| store.remove_attachment(&id, cx));
            }));
        }
        let picker_store = store.clone();
        let add_disabled = is_submitting || slots_left == 0;
        row = row.child(
            plain("appeal-evidence-add")
                .disabled(add_disabled)
                .on_press(move |_, cx| picker_store.update(cx, |store, cx| store.open_picker(cx)))
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
                .opacity(if add_disabled { rise_ui::swiftui::button_style::DISABLED_OPACITY } else { 1.0 })
                .child(symbol("plus", 18.0, secondary))
                .child(div().text_size(px(11.0)).child(tr("report_evidence_add"))),
        );

        vstack(8.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .child(
                hstack(None)
                    .w_full()
                    .px(px(16.0))
                    .child(SecondaryText::new(tr("appeal_evidence_section")).px(13.0))
                    .child(spacer_min_w(8.0))
                    .child(SecondaryText::new(format!("{}/{}", attachments.len(), maximum)).px(13.0)),
            )
            .child(div().id("appeal-evidence-row").w_full().flex().overflow_x_scroll().child(row))
            .child(div().px(px(16.0)).child(SecondaryText::new(tr("appeal_evidence_hint")).px(12.0)))
            .into_any_element()
    }

    fn footer(store: &Entity<AppealSheetStore>, theme: &AppTheme, cx: &App) -> AnyElement {
        let store_value = store.read(cx);
        let is_submitting = store_value.is_submitting();
        let is_valid = store_value.is_valid();
        let submit_store = store.clone();
        div()
            .w_full()
            .px(px(16.0))
            .py(px(10.0))
            .child(
                ButtonUi::new("appeal-sheet-submit")
                    .disabled(is_submitting || !is_valid)
                    .is_pending(is_submitting)
                    .background_color(theme.primary._100)
                    .height(50.0)
                    .loader_color(theme.content_on(theme.primary._100))
                    .on_press(move |_, cx| submit_store.update(cx, |store, cx| store.submit(cx)))
                    .child(MainText::new(tr("appeal_submit")).px(16.0).font_weight(FontWeight::SEMIBOLD).color(theme.content_on(theme.primary._100))),
            )
            .into_any_element()
    }
}

impl Default for AppealSheet {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for AppealSheet {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let store = AppealSheetStore::shared(cx);
        let is_submitting = store.read(cx).is_submitting();

        let reader = store.clone();
        let writer = store.clone();
        let content_store = store.clone();
        let footer_store = store.clone();
        let did_dismiss_store = store.clone();
        let refused_store = store;
        BottomSheetUi::new(
            "appeal-sheet",
            Binding::new(
                move |cx| reader.read(cx).is_sheet_presented(),
                move |presented, _, cx| writer.update(cx, |store, cx| store.set_sheet_presented(presented, cx)),
            ),
            move |_, cx| {
                let theme = rise_ui::theme(cx).clone();
                Self::content(&content_store, &theme, cx)
            },
            move |_, cx| {
                let theme = rise_ui::theme(cx).clone();
                Self::footer(&footer_store, &theme, cx)
            },
        )
        .snap_fractions(vec![0.78, 0.94])
        .full_page_allows_swipe_dismiss(!is_submitting)
        .dismisses_on_backdrop_tap(!is_submitting)
        .dismisses_keyboard_before_backdrop_dismiss(true)
        .footer_shows_top_border(true)
        .content_scrollable(true)
        .sheet_background(Some(theme.bg._100))
        .footer_background(Some(theme.bg._100))
        .on_did_dismiss(move |_, cx| {
            did_dismiss_store.update(cx, |store, cx| store.sheet_did_dismiss(cx));
        })
        .on_presentation_refused(move |_, cx| {
            refused_store.update(cx, |store, cx| store.sheet_presentation_refused(cx));
        })
    }
}

fn symbol(name: &str, side: f32, color: Hsla) -> AnyElement {
    let filled = name.ends_with(".fill");
    match IconUi::sized(name, px(side), color, filled).or_else(|| IconUi::sized(name, px(side), color, !filled)) {
        Some(icon) => icon.into_any_element(),
        None => div().size(px(side)).into_any_element(),
    }
}
