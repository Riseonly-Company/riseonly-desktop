use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::{
    AnyElement, App, Context, ElementId, Entity, ExternalPaths, FontWeight, ImageFormat, IntoElement, KeyDownEvent,
    PromptButton, PromptLevel, Render, RenderOnce, SharedString, Subscription, Window, white,
};
use rise_canon::{RiseBugReportSurface, RiseFeedbackKind, RiseGuideAnchor};
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::{FormatArg, tr, tr_args};
use rise_platform::pasteboard::UIPasteboard;
use rise_platform::uniform_type::UTType;
use rise_theme::{AppTheme, ThemeStore, color};
use rise_ui::IconUi;
use rise_ui::bottom_sheet_ui::bottom_sheet_screen::{
    BottomSheetNavigationState, BottomSheetScreenItem, bottom_sheet_navigation,
};
use rise_ui::button_ui::ButtonUi;
use rise_ui::error_text_ui::ErrorTextUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::input_ui::input_actions::Paste;
use rise_ui::main_text::MainText;
use rise_ui::search_field_ui::SearchFieldUi;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::environment_values::IsEnabledKey;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::progress_view::progress_view_value;

use crate::core::ui::bottom_sheet_ui::bottom_sheet_pages_ui::BottomSheetPagesUi;
use crate::core::ui::clever_image_ui::{CleverImageUi, ContentMode};
use crate::core::ui::input_field_accessory::InputFieldAccessory;
use crate::core::ui::input_ui::InputUi;
use rise_ui::layer_keys::LayerKeysExt;
use crate::core::ui::text_area_ui::TextAreaUi;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierSendOptions};
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_shared_ui::ChatProfileEmptyState;
use crate::modules::diagnostics::engine::core::rise_bug_report_upload::{
    BugReportAttachment, BugReportAttachmentKind, RiseBugReportUpload,
};
use crate::modules::diagnostics::stores::diagnostics::diagnostics_actions::bug_report_types::{
    BugReportDraft, BugReportDraftKind, BugReportSheetPage, BugReportSubmissionState,
};
use crate::modules::diagnostics::stores::diagnostics::diagnostics_interactions::bug_report_interactions::BugReportInteractionsStore;
use crate::modules::diagnostics::stores::diagnostics::diagnostics_services::bug_report_services::BugReportServicesStore;
use crate::modules::media_picker::api::media_picker_result::{MediaPickerItem, MediaPickerItemKind, MediaPickerResult};
use crate::modules::media_picker::export::dropped_video_probe::DroppedVideoProbe;
use crate::modules::media_picker::ui::media_picker_session::MediaPickerSession;

const ESCAPE_KEY: &str = "escape";
const PASTE_KEY: &str = "v";
const CHECK_MARK: &str = "\u{2713}";

#[derive(IntoElement)]
pub struct BugReportSheet {
    services: Entity<BugReportServicesStore>,
    interactions: Entity<BugReportInteractionsStore>,
}

impl BugReportSheet {
    pub fn new(cx: &mut App) -> Self {
        Self { services: BugReportServicesStore::shared(cx), interactions: BugReportInteractionsStore::shared(cx) }
    }

    fn pages(&self, root_content_height: &Entity<f32>, window: &Window, cx: &App) -> Vec<BottomSheetScreenItem> {
        let services = self.services.read(cx);
        let height_writer = root_content_height.clone();
        let areas_title = tr(if services.active_kind() == BugReportDraftKind::Feedback {
            "feedback_where_section"
        } else {
            "bug_report_where_section"
        });
        vec![
            BottomSheetScreenItem::new(
                BugReportSheetPage::Root.raw_value(),
                tr("report_riseonly_title"),
                Self::root_fraction(*root_content_height.read(cx), window),
                move |_: &mut Window, _: &mut App| {
                    let writer = height_writer.clone();
                    BugReportRootPage::new().on_content_height(move |height: f32, cx: &mut App| {
                        writer.update(cx, |root_content_height, cx| {
                            if (*root_content_height - height).abs() > 0.5 {
                                *root_content_height = height;
                                cx.notify();
                            }
                        });
                    })
                },
            ),
            BottomSheetScreenItem::new(
                BugReportSheetPage::Feedback.raw_value(),
                Self::form_title(BugReportDraftKind::Feedback, services),
                0.9,
                |window: &mut Window, cx: &mut App| {
                    window.use_keyed_state("bug-report-form-feedback", cx, |_, cx| {
                        BugReportFormPage::new(BugReportDraftKind::Feedback, cx)
                    })
                },
            ),
            BottomSheetScreenItem::new(
                BugReportSheetPage::Bug.raw_value(),
                Self::form_title(BugReportDraftKind::Bug, services),
                0.9,
                |window: &mut Window, cx: &mut App| {
                    window.use_keyed_state("bug-report-form-bug", cx, |_, cx| {
                        BugReportFormPage::new(BugReportDraftKind::Bug, cx)
                    })
                },
            ),
            BottomSheetScreenItem::new(
                BugReportSheetPage::Areas.raw_value(),
                areas_title,
                0.9,
                |window: &mut Window, cx: &mut App| {
                    window.use_keyed_state("bug-report-areas", cx, |_, cx| BugReportAreasPage::new(cx))
                },
            ),
        ]
    }

    fn root_fraction(root_content_height: f32, window: &Window) -> f32 {
        let screen_height = f32::from(window.viewport_size().height).max(1.0);
        let floor = 0.42_f32.max(300.0 / screen_height);
        if root_content_height <= 0.0 {
            return floor;
        }
        let needed = root_content_height + Self::ROOT_CHROME_HEIGHT + BottomSheetPagesUi::window_bottom_inset();
        0.9_f32.min(floor.max(needed / screen_height))
    }

    const ROOT_CHROME_HEIGHT: f32 = 96.0;

    /// a refile and an edit look like a first submission so the title is the only place to show
    /// the difference
    fn form_title(kind: BugReportDraftKind, services: &BugReportServicesStore) -> String {
        if services.is_editing() {
            return tr("bug_report_my_title");
        }
        tr(if kind == BugReportDraftKind::Feedback { "feedback_title" } else { "bug_report_title" })
    }

    fn present_discard_alert(kind: BugReportDraftKind, window: &mut Window, cx: &mut App) {
        let title = tr(if kind == BugReportDraftKind::Feedback { "feedback_discard_title" } else { "bug_report_discard_title" });
        let answer = window.prompt(
            PromptLevel::Critical,
            &title,
            None,
            &[PromptButton::cancel(tr("cancel")), PromptButton::ok(tr("bug_report_discard_confirm"))],
            cx,
        );
        let interactions = BugReportInteractionsStore::shared(cx);
        cx.spawn(async move |cx| {
            let is_confirmed = matches!(answer.await, Ok(1));
            interactions.update(cx, |interactions, cx| {
                if is_confirmed {
                    interactions.discard_draft(cx);
                } else {
                    interactions.cancel_discard(cx);
                }
            });
        })
        .detach();
    }

    fn handle_capture_key_down(
        services: &Entity<BugReportServicesStore>,
        interactions: &Entity<BugReportInteractionsStore>,
        event: &KeyDownEvent,
        cx: &mut App,
    ) {
        let keystroke = &event.keystroke;
        if keystroke.key == ESCAPE_KEY {
            let store = services.read(cx);
            let asks_first = store.is_sheet_presented()
                && store.has_content()
                && !store.state().is_sending()
                && !store.is_confirming_discard();
            if asks_first {
                cx.stop_propagation();
                interactions.update(cx, |interactions, cx| interactions.request_discard(cx));
            }
        } else if keystroke.key == PASTE_KEY && keystroke.modifiers.secondary() && paste_attachment(services, interactions, cx) {
            cx.stop_propagation();
        }
    }
}

impl RenderOnce for BugReportSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let root_content_height = window.use_keyed_state("bug-report-root-content-height", cx, |_, _| 0.0_f32);
        let (background, pages) = (rise_ui::theme(cx).bg._100, self.pages(&root_content_height, window, cx));
        let (root_page, restored_stack, is_sending, is_confirming, active_kind) = {
            let services = self.services.read(cx);
            (
                services.sheet_root_page(),
                services.restored_page_stack(),
                services.state().is_sending(),
                services.is_confirming_discard(),
                services.active_kind(),
            )
        };
        on_change(
            "bug-report-discard-alert",
            is_confirming,
            false,
            window,
            cx,
            move |_, is_confirming: &bool, window, cx| {
                if *is_confirming {
                    Self::present_discard_alert(active_kind, window, cx);
                }
            },
        );

        let (read_services, write_interactions) = (self.services.clone(), self.interactions.clone());
        let presented = Binding::new(
            move |cx: &App| read_services.read(cx).is_sheet_presented(),
            move |presented: bool, _: &mut Window, cx: &mut App| {
                write_interactions.update(cx, |interactions, cx| interactions.set_sheet_presented(presented, cx));
            },
        );
        let (did_dismiss, refused, page_change) =
            (self.interactions.clone(), self.interactions.clone(), self.interactions.clone());
        let (key_services, key_interactions) = (self.services.clone(), self.interactions.clone());
        let (paste_services, paste_interactions) = (self.services.clone(), self.interactions.clone());
        div()
            .w(px(0.0))
            .h(px(0.0))
            .capture_key_down(move |event: &KeyDownEvent, _: &mut Window, cx: &mut App| {
                Self::handle_capture_key_down(&key_services, &key_interactions, event, cx);
            })
            .capture_action(move |_: &Paste, _: &mut Window, cx: &mut App| {
                if paste_attachment(&paste_services, &paste_interactions, cx) {
                    cx.stop_propagation();
                }
            })
            .child(
                BottomSheetPagesUi::new("bug-report-sheet", presented, pages, root_page.raw_value())
                    .initial_page_ids(restored_stack)
                    .allows_swipe_dismiss(!is_sending)
                    .interaction_locked(is_sending)
                    .sheet_background(Some(background))
                    .on_did_dismiss(move |_: &mut Window, cx: &mut App| {
                        did_dismiss.update(cx, |interactions, cx| interactions.sheet_did_dismiss(cx));
                    })
                    .on_presentation_refused(move |_: &mut Window, cx: &mut App| {
                        refused.update(cx, |interactions, cx| interactions.sheet_presentation_refused(cx));
                    })
                    .on_screen_change(Some(move |id: &str, _: &mut Window, cx: &mut App| {
                        page_change.update(cx, |interactions, cx| interactions.page_did_change(id, cx));
                    })),
            )
    }
}

/// files dropped or pasted go the picker way so the limits are the same
/// video is probed off the main thread first, the draft it was dropped on must still be the
/// open one after
fn attach_external_files(
    services: &Entity<BugReportServicesStore>,
    interactions: &Entity<BugReportInteractionsStore>,
    paths: &[PathBuf],
    cx: &mut App,
) {
    {
        let store = services.read(cx);
        if !store.is_sheet_presented() || !store.can_edit_draft() || !store.can_attach_more() {
            return;
        }
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
                _ => return None,
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
                false,
            ))
        })
        .collect();
    if items.is_empty() {
        return;
    }
    let draft_id = services.read(cx).draft().client_report_id;
    let (services, interactions) = (services.clone(), interactions.clone());
    cx.spawn(async move |cx| {
        let items = DroppedVideoProbe::probing_durations(items).await;
        cx.update(|cx| {
            let store = services.read(cx);
            if !store.is_sheet_presented() || !store.can_edit_draft() || store.draft().client_report_id != draft_id {
                return;
            }
            let result = MediaPickerResult::new(items, false);
            interactions.update(cx, |interactions, cx| interactions.attach(result, cx));
        });
    })
    .detach();
}

/// paste on an open form, files copied in a file manager or an image with no text
/// true when the clipboard was taken, text stays with the focused field
fn paste_attachment(
    services: &Entity<BugReportServicesStore>,
    interactions: &Entity<BugReportInteractionsStore>,
    cx: &mut App,
) -> bool {
    {
        let store = services.read(cx);
        if !store.is_sheet_presented() || store.page().form_kind().is_none() {
            return false;
        }
    }
    let pasteboard = UIPasteboard::general();
    let paths = pasteboard.file_paths(cx);
    if !paths.is_empty() {
        attach_external_files(services, interactions, &paths, cx);
        return true;
    }
    if pasteboard.has_strings(cx) {
        return false;
    }
    let Some(image) = pasteboard.image(cx) else {
        return false;
    };
    if !matches!(
        image.format,
        ImageFormat::Png
            | ImageFormat::Jpeg
            | ImageFormat::Webp
            | ImageFormat::Gif
            | ImageFormat::Tiff
            | ImageFormat::Bmp
    ) {
        return false;
    }
    let (services, interactions) = (services.clone(), interactions.clone());
    let writer = cx.background_executor().spawn(async move { MediaPickerSession::write_pasted_image(&image) });
    cx.spawn(async move |cx| match writer.await {
        Ok(path) => {
            cx.update(|cx| attach_external_files(&services, &interactions, &[path], cx));
        }
        Err(error) => {
            tracing::warn!(target: "riseonly", %error, "pasted image was not staged");
            cx.update(|cx| Notifier::send(tr("sticker_pack_media_unsupported"), NotifierSendOptions::default(), cx));
        }
    })
    .detach();
    true
}

#[derive(IntoElement)]
struct BugReportRootPage {
    on_content_height: Rc<dyn Fn(f32, &mut App)>,
}

impl BugReportRootPage {
    fn new() -> Self {
        Self { on_content_height: Rc::new(|_: f32, _: &mut App| {}) }
    }

    fn on_content_height(mut self, on_content_height: impl Fn(f32, &mut App) + 'static) -> Self {
        self.on_content_height = Rc::new(on_content_height);
        self
    }
}

impl RenderOnce for BugReportRootPage {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (feedback_navigation, bug_navigation) = (bottom_sheet_navigation(cx), bottom_sheet_navigation(cx));
        let on_content_height = self.on_content_height;
        div()
            .flex()
            .flex_col()
            .size_full()
            .pt(px(4.0))
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .w_full()
                    .gap(px(14.0))
                    .child(div().px(px(16.0)).child(SecondaryText::new(tr("report_riseonly_subtitle")).px(13.0)))
                    .child(
                        GroupedBtns::new(
                            "bug-report-root-doors",
                            vec![
                                GroupedBtn::new("report_feedback", tr("report_riseonly_feedback_button"))
                                    .icon("lightbulb.fill")
                                    .icon_background(color("#FF9F0A"))
                                    .subtitle(tr("report_riseonly_feedback_hint"))
                                    .subtitle_line_limit(2)
                                    .callback(move |window: &mut Window, cx: &mut App| {
                                        navigate(&feedback_navigation, BugReportSheetPage::Feedback, window, cx);
                                    }),
                                GroupedBtn::new("report_bug", tr("bug_report_title"))
                                    .icon("ladybug.fill")
                                    .icon_background(color("#FF3B30"))
                                    .subtitle(tr("report_riseonly_bug_hint"))
                                    .subtitle_line_limit(2)
                                    .guide_anchor(RiseGuideAnchor::BugReportBugRow)
                                    .callback(move |window: &mut Window, cx: &mut App| {
                                        navigate(&bug_navigation, BugReportSheetPage::Bug, window, cx);
                                    }),
                            ],
                        )
                        .horizontal_padding(16.0),
                    )
                    .child(measure("bug-report-root-content", move |bounds, _: &mut Window, cx: &mut App| {
                        on_content_height((f32::from(bounds.size.height) + 16.0).ceil(), cx);
                    })),
            )
            .child(div().flex_1().min_h_0())
    }
}

fn navigate(
    navigation: &Option<Entity<BottomSheetNavigationState>>,
    page: BugReportSheetPage,
    window: &mut Window,
    cx: &mut App,
) {
    if let Some(navigation) = navigation {
        BottomSheetNavigationState::navigate(navigation, page.raw_value(), window, cx);
    }
}

/// one form for both kinds, the kind decides one row and the copy and the store decides
/// which draft the fields read
struct BugReportFormPage {
    kind: BugReportDraftKind,
    services: Entity<BugReportServicesStore>,
    interactions: Entity<BugReportInteractionsStore>,
    _observers: [Subscription; 2],
}

impl BugReportFormPage {
    fn new(kind: BugReportDraftKind, cx: &mut Context<Self>) -> Self {
        let services = BugReportServicesStore::shared(cx);
        let interactions = BugReportInteractionsStore::shared(cx);
        let theme_store = ThemeStore::shared(cx);
        let observers = [cx.observe(&services, |_, _, cx| cx.notify()), cx.observe(&theme_store, |_, _, cx| cx.notify())];
        Self { kind, services, interactions, _observers: observers }
    }

    fn id(&self, name: &str) -> SharedString {
        SharedString::from(format!("bug-report-{}-{name}", self.kind.raw_value()))
    }

    fn is_feedback(&self) -> bool {
        self.kind == BugReportDraftKind::Feedback
    }

    /// anchors exist only on the bug form, the guide walks that one
    fn anchor(&self, anchor: RiseGuideAnchor) -> Option<RiseGuideAnchor> {
        (self.kind == BugReportDraftKind::Bug).then_some(anchor)
    }

    fn subtitle(&self, services: &BugReportServicesStore) -> String {
        if services.is_follow_up() {
            return tr("bug_report_refile_hint");
        }
        tr(if self.is_feedback() { "feedback_subtitle" } else { "bug_report_subtitle" })
    }

    fn feedback_kind_row(&self, draft: &BugReportDraft, is_sending: bool) -> AnyElement {
        let items = BugReportServicesStore::ordered_feedback_kinds()
            .iter()
            .copied()
            .map(|option| {
                let selected = draft.feedback_kind == option;
                let is_new_idea = option == RiseFeedbackKind::NewIdea;
                let interactions = self.interactions.clone();
                let mut item = GroupedBtn::new(option.wire(), tr(option.i18n_key()))
                    .icon(if is_new_idea { "sparkles" } else { "arrow.up.forward.app.fill" })
                    .icon_background(color(if is_new_idea { "#5E5CE6" } else { "#30B0C7" }))
                    .subtitle(tr(if is_new_idea {
                        "feedback_kind_new_idea_hint"
                    } else {
                        "feedback_kind_improvement_hint"
                    }))
                    .subtitle_line_limit(2)
                    .disabled(is_sending)
                    .accessibility_selected(selected)
                    .show_chevron(false)
                    .callback(move |_: &mut Window, cx: &mut App| {
                        interactions.update(cx, |interactions, cx| interactions.select_feedback_kind(option, cx));
                    });
                if selected {
                    item = item.trailing(CHECK_MARK);
                }
                item
            })
            .collect();
        GroupedBtns::new(self.id("feedback-kinds"), items)
            .title(tr("feedback_kind_label").to_uppercase())
            .horizontal_padding(16.0)
            .into_any_element()
    }

    fn message_field(
        &self,
        draft: &BugReportDraft,
        message_error: Option<String>,
        can_edit: bool,
        theme: &AppTheme,
    ) -> AnyElement {
        let interactions = self.interactions.clone();
        let placeholder = tr(if self.is_feedback() {
            "feedback_message_placeholder"
        } else {
            "bug_report_message_placeholder"
        });
        let field = TextAreaUi::new(self.id("message"))
            .values(HashMap::from([("message".to_owned(), draft.message.clone())]))
            .name("message")
            .placeholder(placeholder)
            .min_lines(4)
            .max_lines(8)
            .error_message(message_error)
            .rule(BugReportServicesStore::message_rule())
            .set_value(move |_: &str, value: &str, _: &mut Window, cx: &mut App| {
                interactions.update(cx, |interactions, cx| interactions.update_message(value, cx));
            });
        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .px(px(16.0))
            .children(Self::message_accessory(draft, theme))
            .child(env_scope::<IsEnabledKey>(can_edit).child(field))
            .guide_anchor(self.anchor(RiseGuideAnchor::BugReportMessage))
            .into_any_element()
    }

    fn message_accessory(draft: &BugReportDraft, theme: &AppTheme) -> Option<AnyElement> {
        let rule = BugReportServicesStore::message_rule();
        let shortfall = rule.shortfall(&draft.message);
        let remaining = rule.remaining(&draft.message);
        if let Some(shortfall) = shortfall.filter(|shortfall| *shortfall > 0) {
            let minimum = BugReportServicesStore::message_minimum();
            let minimum_arg = i64::try_from(minimum).unwrap_or(i64::MAX);
            return Some(
                div()
                    .flex()
                    .flex_row()
                    .justify_between()
                    .gap(px(8.0))
                    .child(
                        MainText::new(tr_args("form_min_length", &[FormatArg::Int(minimum_arg)]))
                            .px(12.0)
                            .font_weight(FontWeight::MEDIUM)
                            .color(theme.text.secondary),
                    )
                    .child(
                        MainText::new(format!("{}/{minimum}", minimum.saturating_sub(shortfall)))
                            .px(12.0)
                            .font_weight(FontWeight::MEDIUM)
                            .monospaced_digits(true)
                            .color(theme.text.secondary),
                    )
                    .into_any_element(),
            );
        }
        let remaining = remaining.filter(|_| InputFieldAccessory::is_visible(remaining, None))?;
        let tint = if remaining == 0 { theme.semantic.error_100 } else { theme.text.secondary };
        Some(
            div()
                .flex()
                .flex_row()
                .justify_end()
                .w_full()
                .child(
                    MainText::new(remaining.to_string())
                        .px(12.0)
                        .font_weight(FontWeight::MEDIUM)
                        .monospaced_digits(true)
                        .color(tint),
                )
                .into_any_element(),
        )
    }

    fn page_row(&self, draft: &BugReportDraft, is_sending: bool, theme: &AppTheme, cx: &App) -> AnyElement {
        let navigation = bottom_sheet_navigation(cx);
        let mut item = GroupedBtn::new(
            "bug_report_where",
            tr(if self.is_feedback() { "feedback_where_section" } else { "bug_report_where_section" }),
        )
        .icon("square.grid.2x2.fill")
        .icon_background(theme.primary._100)
        .subtitle_line_limit(2)
        .trailing(format!("{}/{}", draft.pages.len(), RiseBugReportUpload::maximum_pages()))
        .disabled(is_sending)
        .show_chevron(true)
        .callback(move |window: &mut Window, cx: &mut App| {
            navigate(&navigation, BugReportSheetPage::Areas, window, cx);
        });
        if let Some(label) = Self::selected_pages_label(draft) {
            item = item.subtitle(label);
        }
        GroupedBtns::new(self.id("where"), vec![item]).horizontal_padding(16.0).into_any_element()
    }

    fn selected_pages_label(draft: &BugReportDraft) -> Option<String> {
        if draft.pages.is_empty() {
            return None;
        }
        Some(draft.pages.iter().map(|page| tr(page.i18n_key())).collect::<Vec<_>>().join(", "))
    }

    fn component_field(&self, draft: &BugReportDraft, component_error: Option<String>, can_edit: bool) -> AnyElement {
        let interactions = self.interactions.clone();
        let field = InputUi::new(self.id("component"))
            .values(HashMap::from([("component".to_owned(), draft.component.clone())]))
            .error_message(component_error)
            .name("component")
            .placeholder(tr("bug_report_component_placeholder"))
            .rule(BugReportServicesStore::component_rule())
            .set_value(move |_: &str, value: &str, _: &mut Window, cx: &mut App| {
                interactions.update(cx, |interactions, cx| interactions.update_component(value, cx));
            });
        div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .px(px(16.0))
            .child(SecondaryText::new(tr("bug_report_component_label")).px(13.0))
            .child(env_scope::<IsEnabledKey>(can_edit).child(field))
            .guide_anchor(self.anchor(RiseGuideAnchor::BugReportComponent))
            .into_any_element()
    }

    fn attachments_section(
        &self,
        draft: &BugReportDraft,
        is_sending: bool,
        can_attach_more: bool,
        theme: &AppTheme,
    ) -> AnyElement {
        let thumbnails = draft
            .screenshots
            .iter()
            .chain(draft.video.iter())
            .map(|attachment| self.attachment_thumbnail(attachment, is_sending, theme));
        div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_between()
                    .gap(px(8.0))
                    .px(px(16.0))
                    .child(SecondaryText::new(tr("bug_report_attachments_section")).px(13.0))
                    .child(
                        SecondaryText::new(format!(
                            "{}/{}",
                            draft.screenshots.len() + usize::from(draft.video.is_some()),
                            RiseBugReportUpload::maximum_screenshots() + 1
                        ))
                        .px(13.0),
                    ),
            )
            .child(
                div().id(self.id("attachments")).flex().overflow_x_scroll().child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_none()
                        .gap(px(8.0))
                        .px(px(16.0))
                        .children(thumbnails)
                        .child(self.add_attachment_button(is_sending, can_attach_more, theme)),
                ),
            )
            .into_any_element()
    }

    fn attachment_thumbnail(&self, attachment: &BugReportAttachment, is_sending: bool, theme: &AppTheme) -> AnyElement {
        let preview: AnyElement = match &attachment.preview_url {
            Some(preview) => CleverImageUi::new(preview.absolute_string().to_owned())
                .content_mode(ContentMode::Fill)
                .corner_radius(10.0)
                .shows_progress_indicator(false)
                .maximum_decode_pixel_dimension(256)
                .shows_failure_icon(false)
                .into_any_element(),
            None => div().size_full().rounded_rect(10.0).bg(theme.bg._200).into_any_element(),
        };
        let mut frame = div().relative().flex_none().size(px(72.0)).rounded_rect(10.0).overflow_hidden().child(preview);
        if attachment.kind == BugReportAttachmentKind::Video {
            frame = frame.child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(2.0))
                    .bg(gpui::black().opacity(0.28))
                    .children(IconUi::sized("play.fill", px(15.0), white(), true))
                    .child(
                        MainText::new(tr("bug_report_attached_video"))
                            .px(10.0)
                            .font_weight(FontWeight::MEDIUM)
                            .color(white()),
                    ),
            );
        }
        let (interactions, attachment_id) = (self.interactions.clone(), attachment.id.clone());
        div()
            .relative()
            .flex_none()
            .size(px(72.0))
            .child(frame)
            .child(
                plain(SharedString::from(format!("bug-report-remove-{}", attachment.id)))
                    .absolute()
                    .top(px(4.0))
                    .right(px(4.0))
                    .disabled(is_sending)
                    .on_press(move |_: &mut Window, cx: &mut App| {
                        interactions.update(cx, |interactions, cx| interactions.remove_attachment(&attachment_id, cx));
                    })
                    .child(
                        div().rounded_full().bg(theme.bg._100).children(IconUi::sized(
                            "xmark.circle.fill",
                            px(18.0),
                            theme.text.primary,
                            true,
                        )),
                    ),
            )
            .into_any_element()
    }

    fn add_attachment_button(&self, is_sending: bool, can_attach_more: bool, theme: &AppTheme) -> AnyElement {
        let interactions = self.interactions.clone();
        plain(self.id("add-attachment"))
            .flex_none()
            .disabled(is_sending || !can_attach_more)
            .on_press(move |_: &mut Window, cx: &mut App| {
                interactions.update(cx, |interactions, cx| interactions.open_attachment_picker(cx));
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(4.0))
                    .size(px(72.0))
                    .rounded_rect(10.0)
                    .stroke_border(theme.text.secondary.opacity(0.35), 1.0)
                    .children(IconUi::sized("plus", px(18.0), theme.text.secondary, false))
                    .child(MainText::new(tr("bug_report_add_media")).px(11.0).color(theme.text.secondary)),
            )
            .guide_anchor(self.anchor(RiseGuideAnchor::BugReportAddMedia))
            .into_any_element()
    }

    /// on the page not in a sheet footer since the deck has no footer slot and the button should
    /// swipe away with the form
    fn footer(
        &self,
        state: &BugReportSubmissionState,
        is_active: bool,
        can_send: bool,
        is_editing: bool,
        has_content: bool,
        theme: &AppTheme,
    ) -> AnyElement {
        let is_sending = state.is_sending();
        let mut footer = div().flex().flex_col().gap(px(8.0)).px(px(16.0)).py(px(12.0)).bg(theme.bg._100);
        if is_active && let BugReportSubmissionState::Sending { progress } = state {
            footer = footer.child(progress_view_value(*progress).tint(theme.primary._100));
        }
        if is_active && let BugReportSubmissionState::Failed { message } = state {
            footer = footer.child(ErrorTextUi::new(message.clone()));
        }
        let send_interactions = self.interactions.clone();
        footer = footer.child(
            ButtonUi::new(self.id("send"))
                .disabled(!is_active || is_sending || !can_send)
                .on_press(move |_: &mut Window, cx: &mut App| {
                    send_interactions.update(cx, |interactions, cx| interactions.send_or_retry(cx));
                })
                .child(
                    MainText::new(Self::send_button_title(state, is_active, is_editing))
                        .px(15.0)
                        .font_weight(FontWeight::SEMIBOLD),
                )
                .guide_anchor(self.anchor(RiseGuideAnchor::BugReportSend)),
        );
        let discard_label = || {
            MainText::new(tr("bug_report_discard_confirm"))
                .px(13.0)
                .font_weight(FontWeight::MEDIUM)
                .color(theme.text.secondary)
        };
        if is_active && has_content && !is_sending {
            let discard_interactions = self.interactions.clone();
            footer = footer.child(
                plain(self.id("discard"))
                    .mt(px(2.0))
                    .flex()
                    .justify_center()
                    .on_press(move |_: &mut Window, cx: &mut App| {
                        discard_interactions.update(cx, |interactions, cx| interactions.request_discard(cx));
                    })
                    .child(discard_label()),
            );
        } else if !(is_active && is_sending) {
            footer = footer.child(div().mt(px(2.0)).flex().justify_center().opacity(0.0).child(discard_label()));
        }
        footer.into_any_element()
    }

    fn send_button_title(state: &BugReportSubmissionState, is_active: bool, is_editing: bool) -> String {
        if is_active && state.is_sending() {
            return tr("bug_report_sending");
        }
        if is_active && matches!(state, BugReportSubmissionState::Failed { .. }) {
            return tr("retry");
        }
        tr(if is_editing { "save" } else { "bug_report_send" })
    }
}

impl Render for BugReportFormPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let services = self.services.read(cx);
        let is_active = services.draft().kind == self.kind;
        let draft = if is_active { services.draft() } else { services.draft_for(self.kind) };
        let state = services.state().clone();
        let is_sending = state.is_sending();
        let subtitle = self.subtitle(services);
        let can_edit = services.can_edit_draft();
        let can_attach_more = services.can_attach_more();
        let can_send = services.can_send();
        let is_editing = services.is_editing();
        let has_content = services.has_content();
        let message_error = if is_active { services.message_error() } else { None };
        let component_error = if is_active { services.component_error() } else { None };

        let mut column = div()
            .flex()
            .flex_col()
            .w_full()
            .gap(px(18.0))
            .pt(px(8.0))
            .pb(px(16.0))
            .child(div().px(px(16.0)).child(SecondaryText::new(subtitle).px(13.0)));
        if self.is_feedback() {
            column = column.child(self.feedback_kind_row(&draft, is_sending));
        }
        column = column
            .child(self.message_field(&draft, message_error, can_edit, &theme))
            .child(self.page_row(&draft, is_sending, &theme, cx));
        if !self.is_feedback() {
            column = column.child(self.component_field(&draft, component_error, can_edit));
        }
        column = column.child(self.attachments_section(&draft, is_sending, can_attach_more, &theme));

        let (drop_services, drop_interactions) = (self.services.clone(), self.interactions.clone());
        let submit_interactions = self.interactions.clone();
        div()
            .flex()
            .flex_col()
            .size_full()
            .form_primary(is_active && !is_sending && can_send, move |_: &mut Window, cx: &mut App| {
                submit_interactions.update(cx, |interactions, cx| interactions.send_or_retry(cx));
            })
            .on_drop(move |paths: &ExternalPaths, _: &mut Window, cx: &mut App| {
                attach_external_files(&drop_services, &drop_interactions, paths.paths(), cx);
            })
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(scroll_view(self.id("scroll")).size_full().shows_indicators(false).child(column)),
            )
            .child(self.footer(&state, is_active, can_send, is_editing, has_content, &theme))
    }
}

struct BugReportAreasPage {
    services: Entity<BugReportServicesStore>,
    interactions: Entity<BugReportInteractionsStore>,
    _observers: [Subscription; 2],
}

impl BugReportAreasPage {
    fn new(cx: &mut Context<Self>) -> Self {
        let services = BugReportServicesStore::shared(cx);
        let interactions = BugReportInteractionsStore::shared(cx);
        let theme_store = ThemeStore::shared(cx);
        let observers = [cx.observe(&services, |_, _, cx| cx.notify()), cx.observe(&theme_store, |_, _, cx| cx.notify())];
        Self { services, interactions, _observers: observers }
    }

    fn page_item(&self, surface: RiseBugReportSurface, selected: bool, page_slots_left: i64) -> GroupedBtn {
        let interactions = self.interactions.clone();
        let mut item = GroupedBtn::new(surface.wire(), tr(surface.i18n_key()))
            .disabled(!selected && page_slots_left == 0)
            .accessibility_selected(selected)
            .show_chevron(false)
            .callback(move |_: &mut Window, cx: &mut App| {
                interactions.update(cx, |interactions, cx| interactions.toggle_page(surface, cx));
            });
        if selected {
            item = item.trailing(CHECK_MARK);
        }
        item
    }
}

impl Render for BugReportAreasPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let services = self.services.read(cx);
        let pages = services.visible_pages();
        let draft_pages = services.draft().pages;
        let page_slots_left = services.page_slots_left();

        let (read_services, write_interactions) = (self.services.clone(), self.interactions.clone());
        let query = Binding::new(
            move |cx: &App| read_services.read(cx).page_query().to_owned(),
            move |value: String, _: &mut Window, cx: &mut App| {
                write_interactions.update(cx, |interactions, cx| interactions.update_page_query(&value, cx));
            },
        );

        let list: AnyElement = if pages.is_empty() {
            div()
                .flex()
                .justify_center()
                .w_full()
                .pt(px(24.0))
                .child(ChatProfileEmptyState::new("magnifyingglass", tr("settings_language_empty_title")))
                .into_any_element()
        } else {
            let items = pages
                .iter()
                .map(|surface| self.page_item(*surface, draft_pages.contains(surface), page_slots_left))
                .collect();
            GroupedBtns::new(ElementId::Name("bug-report-areas-list".into()), items)
                .title(format!("{}/{}", draft_pages.len(), RiseBugReportUpload::maximum_pages()))
                .horizontal_padding(16.0)
                .separator_leading_padding(16.0)
                .into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .pt(px(4.0))
            .child(
                div()
                    .px(px(16.0))
                    .pb(px(12.0))
                    .child(SearchFieldUi::new("bug-report-areas-search", query).uses_inline_text_field(true)),
            )
            .child(
                div().flex_1().min_h_0().child(
                    scroll_view("bug-report-areas-scroll")
                        .size_full()
                        .shows_indicators(false)
                        .child(div().flex().flex_col().w_full().child(list).child(div().min_h(px(40.0)))),
                ),
            )
    }
}
