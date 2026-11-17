use std::cell::Cell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::LazyLock;

use gpui::{
    App, Context, ElementId, Entity, ExternalPaths, FocusHandle, Image, KeyDownEvent, ListAlignment, ListOffset,
    SharedString, Subscription, Window,
};
use rise_canon::RiseLimits;
use rise_core::foundation::foundation_calendar::Calendar;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_platform::pasteboard::UIPasteboard;
use rise_platform::uniform_type::UTType;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::grouped_btns::GroupedCardMetrics;
use rise_ui::input_ui::{Confirm, Paste};
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::swiftui::page_refresh::PageFocus;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::ui::text_area_ui::TextAreaUi;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::core::widgets::sheets::report_sheet::ReportEvidenceThumbnail;
use crate::core::widgets::wrappers::list_shell_ui::{
    ListShellScrollUnderChromeModifiers, ListShellUi, list_shell_floating_top_inset,
};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::message_row::MessageRow;
use crate::modules::chat::components::chat::message_system_pill::MessageSystemPill;
use crate::modules::chat::shared::chat_saved_message_presentation::ChatMessageGrouping;
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType};
use crate::modules::chat::stores::message::message_actions::message_types::{GetMessageMessage, MessageContentType};
use crate::modules::diagnostics::engine::core::rise_bug_report_upload::{BugReportAttachment, BugReportAttachmentKind};
use crate::modules::diagnostics::stores::diagnostics::diagnostics_interactions::bug_report_interactions::BugReportAttachmentFactory;
use crate::modules::media_picker::api::media_picker_configuration::MediaPickerConfiguration;
use crate::modules::media_picker::api::media_picker_result::{MediaPickerItem, MediaPickerItemKind, MediaPickerResult};
use crate::modules::media_picker::ui::media_picker_session::MediaPickerSession;
use crate::modules::media_picker::ui::media_picker_sheet::MediaPickerSheet;
use crate::modules::moderation::components::moderation_case_ui::RemoteEvidenceStrip;
use crate::modules::moderation::components::moderation_kit_ui::{
    ModerationCaseSkeleton, ModerationEmptyState, moderation_child_id, moderation_symbol,
};
use crate::modules::moderation::engine::core::report_evidence_item::ReportEvidenceItem;
use crate::modules::moderation::engine::core::rise_report_evidence_upload::RiseReportEvidenceUpload;
use crate::modules::moderation::engine::rise_moderation_presentation::RiseModerationPresentation;
use crate::modules::moderation::engine::rise_moderation_wire::RiseStoredModerationNote;
use crate::modules::moderation::stores::moderation_interactions::moderation_interactions::ModerationInteractionsStore;

static NOTE_RULE: LazyLock<RiseFieldRule> = LazyLock::new(|| RiseFieldRule::new("report_add_report_note", "/text"));
static NOTE_CEILING: LazyLock<i64> = LazyLock::new(|| RiseLimits::service_value("moderation_note_chars"));
const EVIDENCE_TILE: f32 = 132.0;
const EVIDENCE_SPACING: f32 = 8.0;
const EVIDENCE_MAX_WIDTH: f32 = 260.0;
const NOTE_FIELD: &str = "note";
const RETURN_KEY: &str = "enter";
const PASTE_KEY: &str = "v";
const COMPOSER_BUTTON_SIDE: f32 = 40.0;

fn clamped_note(value: &str) -> String {
    let ceiling = usize::try_from(*NOTE_CEILING).unwrap_or(0);
    if value.chars().count() <= ceiling {
        return value.to_owned();
    }
    value.chars().take(ceiling).collect()
}

fn evidence_strip_width(count: usize) -> f32 {
    let tiles = count.max(1) as f32;
    EVIDENCE_MAX_WIDTH.min(tiles * EVIDENCE_TILE + (tiles - 1.0) * EVIDENCE_SPACING)
}

fn external_item(path: &Path) -> Option<MediaPickerItem> {
    if !path.is_file() {
        return None;
    }
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
}

struct CaseNotesPageState {
    draft: String,
    attachments: Vec<BugReportAttachment>,
    is_picker_presented: bool,
    list: ListUi,
    list_ids: Vec<String>,
    focus: FocusHandle,
    line_break_pending: Rc<Cell<bool>>,
    _observations: Vec<Subscription>,
}

impl CaseNotesPageState {
    fn new(cx: &mut Context<Self>) -> Self {
        let presentation = RiseModerationPresentation::shared(cx);
        let auth = AuthServicesStore::shared(cx);
        Self {
            draft: String::new(),
            attachments: Vec::new(),
            is_picker_presented: false,
            list: ListUi::new(rise_ui::theme(cx), 0, ListAlignment::Top),
            list_ids: Vec::new(),
            focus: cx.focus_handle(),
            line_break_pending: Rc::new(Cell::new(false)),
            _observations: vec![
                cx.observe(&presentation, |_, _, cx| cx.notify()),
                cx.observe(&auth, |_, _, cx| cx.notify()),
            ],
        }
    }

    fn sync_list(&mut self, ids: Vec<String>) {
        if ids == self.list_ids {
            return;
        }
        if !self.list_ids.is_empty() && ids.len() > self.list_ids.len() && ids.starts_with(&self.list_ids) {
            self.list.appended(ids.len() - self.list_ids.len());
        } else {
            self.list.replaced(ids.len());
        }
        self.list_ids = ids;
    }

    fn slots_left(&self) -> i64 {
        (RiseReportEvidenceUpload::maximum_attachments() - self.attachments.len() as i64).max(0)
    }

    fn remove_attachment(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(index) = self.attachments.iter().position(|attachment| attachment.id == id) else {
            return;
        };
        let removed = self.attachments.remove(index);
        let urls: Vec<Url> = [removed.url, removed.preview_url].into_iter().flatten().collect();
        BugReportAttachmentFactory::remove_temporary_files(&urls);
        cx.notify();
    }

    fn attach(&mut self, result: MediaPickerResult, cx: &mut Context<Self>) {
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
        cx.notify();
    }

    /// desktop, files dropped on the thread or pasted into it
    fn attach_external_files(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        if self.slots_left() <= 0 {
            return;
        }
        let items: Vec<MediaPickerItem> = paths.iter().filter_map(|path| external_item(path)).collect();
        if items.is_empty() {
            return;
        }
        self.attach(MediaPickerResult::new(items, false), cx);
    }

    fn attach_pasted_image(&mut self, image: Image, cx: &mut Context<Self>) {
        if self.slots_left() <= 0 {
            return;
        }
        cx.spawn(async move |this, cx| {
            let written =
                cx.background_executor().spawn(async move { MediaPickerSession::write_pasted_image(&image) }).await;
            let _ = this.update(cx, |this, cx| match written {
                Ok(path) => this.attach_external_files(&[path], cx),
                Err(error) => {
                    tracing::warn!(target: "riseonly", %error, "a pasted image was not staged");
                    Notifier::send(tr("sticker_pack_media_unsupported"), NotifierSendOptions::default(), cx);
                }
            });
        })
        .detach();
    }

    fn send(&mut self, resolved_id: Option<i64>, is_sending: bool, cx: &mut Context<Self>) {
        let Some(id) = resolved_id else {
            return;
        };
        if !self.can_send(is_sending) {
            return;
        }
        let text = self.draft.clone();
        let files = self.attachments.clone();
        let sent = ModerationInteractionsStore::shared(cx).update(cx, |store, cx| store.add_note(id, &text, files, cx));
        cx.spawn(async move |this, cx| {
            if sent.await {
                let _ = this.update(cx, |this, cx| {
                    this.draft.clear();
                    this.attachments.clear();
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn can_send(&self, is_sending: bool) -> bool {
        if is_sending {
            return false;
        }
        !trimming_whitespaces_and_newlines(&self.draft).is_empty() || !self.attachments.is_empty()
    }
}

/// desktop, what cmd+v or ctrl+v takes from the clipboard for the thread
/// true if it was a file or an image, plain text stays with the field
fn attach_pasteboard(state: &Entity<CaseNotesPageState>, cx: &mut App) -> bool {
    let paths = UIPasteboard::general().file_paths(cx);
    if !paths.is_empty() {
        state.update(cx, |state, cx| state.attach_external_files(&paths, cx));
        return true;
    }
    let Some(image) = UIPasteboard::general().image(cx) else {
        return false;
    };
    state.update(cx, |state, cx| state.attach_pasted_image(image, cx));
    true
}

/// a group so a colleagues note carries their name and avatar
/// own notes take their side from is_own on the row
fn host(report_id: &str) -> ChatInfo {
    let mut chat = ChatInfo::new(format!("moderation_notes_{report_id}"));
    chat.title = tr("case_notes_section");
    chat.r#type = ChatType::Group;
    chat
}

fn adapt(note: &RiseStoredModerationNote, chat_id: &str) -> GetMessageMessage {
    let mut message = GetMessageMessage::new(note.id.to_string());
    message.chat_id = chat_id.to_owned();
    message.sender_id = note.author_id.clone();
    message.sender_name = note.author.as_ref().map(|author| author.display_name()).unwrap_or_else(|| note.author_id.clone());
    message.sender_tag = note.author.as_ref().and_then(|author| author.tag.clone()).unwrap_or_default();
    message.sender_logo = note.author.as_ref().and_then(|author| author.logo.clone());
    message.sender_is_official = note.author.as_ref().is_some_and(|author| author.is_official);
    message.created_at = note.created_at_ms as f64 / 1000.0;
    message.content_type = MessageContentType::Text;
    message.text = note.text.clone();
    message
}

fn day_separator(messages: &[GetMessageMessage], index: usize) -> Option<String> {
    let calendar = Calendar::current();
    let day_of = |message: &GetMessageMessage| {
        calendar.start_of_day(&Date::from_time_interval_since_1970(message.created_at))
    };
    let day = day_of(messages.get(index)?);
    if index > 0 && day_of(&messages[index - 1]) == day {
        return None;
    }
    Some(RiseFormat::day_label(&day, &Date::now(), &calendar, &RiseFormat::app_locale(), false))
}

fn notes_element_id(report_id: &str, name: impl Into<SharedString>) -> ElementId {
    moderation_child_id(&ElementId::Name(SharedString::from(format!("case-notes-{report_id}"))), name)
}

/// internal moderator notes for one case drawn as a thread
/// nothing here is visible outside moderation so no wallpaper and no delivery state
/// borrows only the message bubble
#[derive(IntoElement)]
pub struct CaseNotesPage {
    report_id: String,
}

impl CaseNotesPage {
    pub fn new(report_id: impl Into<String>) -> Self {
        Self { report_id: report_id.into() }
    }

    fn element_id(&self, name: impl Into<SharedString>) -> ElementId {
        notes_element_id(&self.report_id, name)
    }

    fn picker(&self, state: &Entity<CaseNotesPageState>, cx: &mut App) -> impl IntoElement {
        let reader = state.clone();
        let writer = state.clone();
        let completed = state.clone();
        let slots_left = state.read(cx).slots_left();
        div().absolute().top_0().left_0().size_full().child(
            MediaPickerSheet::new(
                self.element_id("picker"),
                Binding::new(
                    move |cx| reader.read(cx).is_picker_presented,
                    move |presented, _, cx| {
                        writer.update(cx, |state, cx| {
                            state.is_picker_presented = presented;
                            cx.notify();
                        });
                    },
                ),
                MediaPickerConfiguration::report_evidence_slots_left(slots_left),
                ThemeStore::shared(cx),
                move |result: MediaPickerResult, _: &mut Window, cx: &mut App| {
                    let state = completed.clone();
                    cx.defer(move |cx| state.update(cx, |state, cx| state.attach(result, cx)));
                },
            )
            .experimental_bottom_sheet(true),
        )
    }
}

impl RenderOnce for CaseNotesPage {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let background = rise_ui::theme(cx).bg._100;
        let nav_store = env::<NavigationStoreKey>(cx);
        let state = window.use_keyed_state(self.element_id("state"), cx, |_, cx| CaseNotesPageState::new(cx));
        let resolved_id = self.report_id.parse::<i64>().ok();
        let presentation = RiseModerationPresentation::shared(cx);
        let value = resolved_id.and_then(|id| presentation.read(cx).cases().get(&id).cloned());
        let is_loaded = value.is_some();
        let notes = value.map(|value| value.notes).unwrap_or_default();
        let is_sending = resolved_id.is_some_and(|id| presentation.read(cx).noting_reports().contains(&id));
        let subtitle = (!notes.is_empty()).then(|| SharedString::from(RiseFormat::compact_count(notes.len() as i64)));
        let is_picker_presented = state.read(cx).is_picker_presented;
        let focus = state.read(cx).focus.clone();
        let page_focus = PageFocus::keyed_with(self.element_id("page-focus"), focus.clone(), window, cx);
        let leaving_focus = page_focus.clone();
        let key_state = state.clone();
        let drop_state = state.clone();

        let shell = ListShellUi::new()
            .title(SharedString::from(tr("case_notes_section")))
            .subtitle(subtitle)
            .shows_back_button(true)
            .on_back(move |_, cx| {
                if let Some(nav_store) = &nav_store {
                    nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
                }
            })
            .respects_keyboard_safe_area(true)
            .content(
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .bg(background)
                    .child(div().w_full().flex_1().min_h(px(0.0)).child(CaseNotesThread {
                        report_id: self.report_id.clone(),
                        is_loaded,
                        notes,
                        state: state.clone(),
                    }))
                    .child(CaseNotesComposer {
                        report_id: self.report_id.clone(),
                        resolved_id,
                        is_sending,
                        state: state.clone(),
                    }),
            );

        div()
            .id(self.element_id("page"))
            .size_full()
            .relative()
            .track_focus(&focus)
            .on_key_down(move |event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == PASTE_KEY
                    && event.keystroke.modifiers.secondary()
                    && attach_pasteboard(&key_state, cx)
                {
                    cx.stop_propagation();
                }
            })
            .on_drop(move |paths: &ExternalPaths, _, cx| {
                drop_state.update(cx, |state, cx| state.attach_external_files(paths.paths(), cx));
            })
            .child(shell)
            .when(is_picker_presented, |page| page.child(self.picker(&state, cx)))
            .on_appear(self.element_id("task"), move |window, cx| {
                if let Some(id) = resolved_id {
                    ModerationInteractionsStore::shared(cx).update(cx, |store, cx| {
                        store.open_case(id, cx);
                        store.case_screen_opened(id);
                    });
                }
                PageFocus::did_appear(&page_focus, window, cx);
            })
            .on_disappear(self.element_id("closed"), move |window, cx| {
                if let Some(id) = resolved_id {
                    ModerationInteractionsStore::shared(cx).update(cx, |store, _| store.case_screen_closed(id));
                }
                if let Some(window) = window {
                    PageFocus::will_disappear(&leaving_focus, window, cx);
                }
            })
    }
}

#[derive(IntoElement)]
struct CaseNotesThread {
    report_id: String,
    is_loaded: bool,
    notes: Vec<RiseStoredModerationNote>,
    state: Entity<CaseNotesPageState>,
}

impl RenderOnce for CaseNotesThread {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        if !self.is_loaded {
            return scroll_view(notes_element_id(&self.report_id, "skeleton"))
                .scroll_disabled(true)
                .list_shell_scroll_under_chrome(cx)
                .child(ModerationCaseSkeleton::new())
                .into_any_element();
        }
        if self.notes.is_empty() {
            return scroll_view(notes_element_id(&self.report_id, "empty"))
                .list_shell_scroll_under_chrome(cx)
                .child(ModerationEmptyState::new("note.text", tr("case_notes_empty"), theme))
                .into_any_element();
        }

        let chat = host(&self.report_id);
        let messages: Vec<GetMessageMessage> = self.notes.iter().map(|note| adapt(note, &chat.id)).collect();
        let evidence_by_note: HashMap<String, Vec<ReportEvidenceItem>> =
            self.notes.iter().map(|note| (note.id.to_string(), note.evidence())).collect();
        let flags = ChatMessageGrouping::flags(&messages, &chat, cx);
        let auth = AuthServicesStore::shared(cx);
        let me = auth.read(cx).effective_user_id_for_wire().unwrap_or_default();
        let top_inset = list_shell_floating_top_inset(cx);
        let ids: Vec<String> = messages.iter().map(|message| message.id.clone()).collect();
        let last = ids.last().cloned();
        let item_count = messages.len();
        self.state.update(cx, |state, _| state.sync_list(ids));

        let list_state = self.state.read(cx).list.state().clone();
        on_change(
            notes_element_id(&self.report_id, "open-on-last"),
            last,
            true,
            window,
            cx,
            move |_, _, _, _| {
                list_state.scroll_to(ListOffset { item_ix: item_count.saturating_sub(1), offset_in_item: px(0.0) });
            },
        );

        let report_id = self.report_id;
        self.state
            .read(cx)
            .list
            .element(move |index, _, _| {
                let Some(message) = messages.get(index) else {
                    return div().into_any_element();
                };
                let (is_first, is_last) = flags.get(index).map_or((true, true), |group| (group.is_first, group.is_last));
                let is_own = message.sender_id == me;
                let evidence = evidence_by_note.get(&message.id).filter(|evidence| !evidence.is_empty()).cloned();
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .items_start()
                    .pt(px(if index == 0 { top_inset + 12.0 } else { 0.0 }))
                    .pb(px(if index + 1 == item_count { 12.0 } else { MessageSizes::list_row_spacing() }))
                    .when_some(day_separator(&messages, index), |column, label| {
                        column.child(div().w_full().pt(px(6.0)).pb(px(2.0)).child(MessageSystemPill::new(label)))
                    })
                    .child(
                        div()
                            .w_full()
                            .pt(px(if is_first {
                                MessageSizes::list_first_in_group_top_padding()
                            } else {
                                MessageSizes::list_continuation_top_padding()
                            }))
                            .pb(px(if is_last { MessageSizes::list_last_in_group_bottom_padding() } else { 0.0 }))
                            .child(
                                MessageRow::new(
                                    message.clone(),
                                    chat.clone(),
                                    is_own,
                                    is_first,
                                    is_last,
                                    !is_own && is_last,
                                )
                                .id(notes_element_id(&report_id, format!("message-{}", message.id)))
                                .draws_inline_avatar(!is_own && is_last)
                                .allows_reply(false)
                                .interactions_enabled(false)
                                .allows_composer_actions(false),
                            ),
                    )
                    .when_some(evidence, |column, evidence| {
                        let count = evidence.len();
                        column.child(
                            hstack(0.0)
                                .w_full()
                                .px(px(MessageSizes::row_side_inset() + 8.0))
                                .pb(px(6.0))
                                .when(is_own, |row| row.child(spacer_min_w(0.0)))
                                .child(
                                    div().w(px(evidence_strip_width(count))).child(
                                        RemoteEvidenceStrip::new(
                                            notes_element_id(&report_id, format!("evidence-{}", message.id)),
                                            evidence,
                                        )
                                        .size(EVIDENCE_TILE),
                                    ),
                                )
                                .when(!is_own, |row| row.child(spacer_min_w(0.0))),
                        )
                    })
                    .into_any_element()
            })
            .into_any_element()
    }
}

#[derive(IntoElement)]
struct CaseNotesComposer {
    report_id: String,
    resolved_id: Option<i64>,
    is_sending: bool,
    state: Entity<CaseNotesPageState>,
}

impl CaseNotesComposer {
    fn element_id(&self, name: impl Into<SharedString>) -> ElementId {
        notes_element_id(&self.report_id, name)
    }

    fn attachments_row(&self, attachments: Vec<BugReportAttachment>) -> impl IntoElement {
        let is_sending = self.is_sending;
        let thumbnails: Vec<AnyElement> = attachments
            .into_iter()
            .map(|attachment| {
                let state = self.state.clone();
                let id = attachment.id.clone();
                ReportEvidenceThumbnail::new(attachment, is_sending, move |_, cx| {
                    state.update(cx, |state, cx| state.remove_attachment(&id, cx));
                })
                .into_any_element()
            })
            .collect();
        div()
            .id(self.element_id("attachments"))
            .w_full()
            .flex()
            .overflow_x_scroll()
            .child(hstack(8.0).flex_none().px(px(2.0)).children(thumbnails))
    }

    fn attach_button(&self, slots_left: i64, theme: &AppTheme) -> impl IntoElement {
        let state = self.state.clone();
        plain(self.element_id("attach"))
            .disabled(slots_left == 0 || self.is_sending)
            .on_press(move |_, cx| {
                state.update(cx, |state, cx| {
                    state.is_picker_presented = true;
                    cx.notify();
                });
            })
            .size(px(COMPOSER_BUTTON_SIDE))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .opacity(if slots_left == 0 { 0.4 } else { 1.0 })
            .children(moderation_symbol("paperclip", 17.0, theme.text.secondary))
    }

    fn send_button(&self, can_send: bool, theme: &AppTheme) -> impl IntoElement {
        let state = self.state.clone();
        let resolved_id = self.resolved_id;
        let is_sending = self.is_sending;
        let fill = theme.primary._100;
        let content = theme.content_on(fill);
        div().flex_none().pb(px(3.0)).child(
            plain(self.element_id("send"))
                .disabled(!can_send)
                .on_press(move |_, cx| {
                    state.update(cx, |state, cx| state.send(resolved_id, is_sending, cx));
                })
                .size(px(COMPOSER_BUTTON_SIDE))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(fill)
                .opacity(if can_send { 1.0 } else { 0.5 })
                .when(is_sending, |button| button.child(progress_view().tint(content)))
                .when(!is_sending, |button| button.children(moderation_symbol("arrow.up", 16.0, content))),
        )
    }
}

impl RenderOnce for CaseNotesComposer {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let (draft, attachments, slots_left, can_send, line_break_pending) = {
            let current = self.state.read(cx);
            (
                current.draft.clone(),
                current.attachments.clone(),
                current.slots_left(),
                current.can_send(self.is_sending),
                current.line_break_pending.clone(),
            )
        };

        let draft_writer = self.state.clone();
        let confirm_pending = line_break_pending.clone();
        let confirm_state = self.state.clone();
        let paste_state = self.state.clone();
        let resolved_id = self.resolved_id;
        let is_sending = self.is_sending;

        div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .px(px(GroupedCardMetrics::HORIZONTAL_PADDING))
            .py(px(10.0))
            .bg(theme.bg._100)
            .when(!attachments.is_empty(), |column| column.child(self.attachments_row(attachments)))
            .child(
                hstack(10.0)
                    .w_full()
                    .items_end()
                    .capture_key_down(move |event: &KeyDownEvent, window, cx| {
                        if event.keystroke.key != RETURN_KEY || !event.keystroke.modifiers.shift {
                            return;
                        }
                        line_break_pending.set(true);
                        cx.stop_propagation();
                        window.dispatch_action(Box::new(Confirm), cx);
                    })
                    .capture_action(move |_: &Confirm, _, cx| {
                        if confirm_pending.replace(false) {
                            return;
                        }
                        cx.stop_propagation();
                        confirm_state.update(cx, |state, cx| state.send(resolved_id, is_sending, cx));
                    })
                    .capture_action(move |_: &Paste, _, cx| {
                        if attach_pasteboard(&paste_state, cx) {
                            cx.stop_propagation();
                        }
                    })
                    .child(self.attach_button(slots_left, &theme))
                    .child(
                        div().flex_1().min_w(px(0.0)).child(
                            TextAreaUi::new(self.element_id("note"))
                                .values(HashMap::from([(NOTE_FIELD.to_owned(), draft)]))
                                .name(NOTE_FIELD)
                                .placeholder(tr("case_note_placeholder"))
                                .min_lines(1)
                                .max_lines(5)
                                .rule(*NOTE_RULE)
                                .max_chars(usize::try_from(*NOTE_CEILING).ok())
                                .set_value(move |_, value, _, cx| {
                                    let value = clamped_note(value);
                                    draft_writer.update(cx, |state, cx| {
                                        state.draft = value;
                                        cx.notify();
                                    });
                                }),
                        ),
                    )
                    .child(self.send_button(can_send, &theme)),
            )
    }
}
