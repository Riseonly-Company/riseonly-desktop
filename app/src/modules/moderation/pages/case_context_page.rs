use std::sync::Arc;

use gpui::{App, Context, ElementId, Entity, ScrollHandle, SharedString, Subscription, Window, point, transparent_black};
use rise_canon::rise_key::{events, report_snapshot};
use rise_core::foundation::foundation_calendar::Calendar;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::app::router::navigation_store::{NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, ListShellUi};
use crate::modules::chat::components::chat::backdrop::chat_decorative_backdrop::ChatDecorativeBackdrop;
use crate::modules::chat::components::chat::chat_logo::ChatLogo;
use crate::modules::chat::components::chat::chat_title_row::ChatTitleRow;
use crate::modules::chat::components::chat::media_viewer::chat_media_presentation::{
    ChatGalleryMessagesKey, ChatGalleryPaginatesKey,
};
use crate::modules::chat::components::chat::message_row::MessageRow;
use crate::modules::chat::components::chat::message_system_pill::MessageSystemPill;
use crate::modules::chat::shared::chat_saved_message_presentation::ChatMessageGrouping;
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType};
use crate::modules::chat::stores::message::message_actions::message_types::{GetMessageMessage, MessageContentType};
use crate::modules::moderation::components::moderation_kit_ui::{
    ModerationCaseSkeleton, ModerationEmptyState, ModerationMetrics, moderation_child_id,
};
use crate::modules::moderation::engine::rise_moderation_presentation::RiseModerationPresentation;
use crate::modules::moderation::engine::rise_moderation_wire::{RiseStoredCaseContextMessage, RiseStoredReportCase};
use crate::modules::moderation::stores::moderation_interactions::moderation_interactions::ModerationInteractionsStore;

struct CaseContextPageState {
    scroll: ScrollHandle,
    did_center_reported: bool,
    _observation: Subscription,
}

impl CaseContextPageState {
    fn new(cx: &mut Context<Self>) -> Self {
        let presentation = RiseModerationPresentation::shared(cx);
        Self {
            scroll: ScrollHandle::new(),
            did_center_reported: false,
            _observation: cx.observe(&presentation, |_, _, cx| cx.notify()),
        }
    }
}

/// conversation around a reported message
/// frozen window of up to ten messages each way, captured with the reporters own rights at filing
/// so this page renders a snapshot and never talks to the chat service
#[derive(IntoElement)]
pub struct CaseContextPage {
    report_id: String,
}

impl CaseContextPage {
    pub fn new(report_id: impl Into<String>) -> Self {
        Self { report_id: report_id.into() }
    }

    fn element_id(&self, name: impl Into<SharedString>) -> ElementId {
        context_element_id(&self.report_id, name)
    }

    /// opens the chat profile and not the chat so the moderator doesnt land in someones chat by accident
    fn header(&self, value: &RiseStoredReportCase, cx: &App) -> impl IntoElement {
        let chat = synthetic_chat(value);
        let chat_id = value.report.chat_id.clone().unwrap_or_default();
        let secondary = rise_ui::theme(cx).text.secondary;
        let nav_store = env::<NavigationStoreKey>(cx);
        let is_disabled = chat_id.is_empty();
        plain(self.element_id("header"))
            .disabled(is_disabled)
            .on_press(move |_, cx| {
                if chat_id.is_empty() {
                    return;
                }
                if let Some(nav_store) = &nav_store {
                    let route = RootRoute::ChatProfile { chat_id: Some(chat_id.clone()) };
                    nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
                }
            })
            .child(
                hstack(9.0)
                    .flex_none()
                    .child(
                        ChatLogo::new(self.element_id("header-logo"), chat.clone())
                            .size(34.0)
                            .icon_size(14.0)
                            .show_online_indicator(false)
                            .show_story_ring(false)
                            .letter_fallback(chat.title.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_start()
                            .gap(px(1.0))
                            .child(ChatTitleRow::new(chat).title_px(15.0).icon_size(11.0))
                            .child(
                                MainText::new(tr("case_context_section"))
                                    .px(11.0)
                                    .color(secondary)
                                    .number_of_lines(1),
                            ),
                    )
                    .liquid_glass_nav_pill(10.0, None),
            )
    }
}

fn context_element_id(report_id: &str, name: impl Into<SharedString>) -> ElementId {
    moderation_child_id(&ElementId::Name(SharedString::from(format!("case-context-{report_id}"))), name)
}

/// snapshot has the title and type but no logo so the row falls back to a lettered avatar
fn synthetic_chat(value: &RiseStoredReportCase) -> ChatInfo {
    let facts = &value.report.snapshot_facts;
    let mut chat = ChatInfo::new(value.report.chat_id.clone().unwrap_or_default());
    chat.title = facts
        .string(report_snapshot::CHAT_TITLE)
        .filter(|title| !trimming_whitespaces_and_newlines(title).is_empty())
        .or_else(|| value.report.snapshot_title.clone().filter(|title| !trimming_whitespaces_and_newlines(title).is_empty()))
        .or_else(|| value.report.reported_user.as_ref().map(|user| user.display_name()).filter(|name| !trimming_whitespaces_and_newlines(name).is_empty()))
        .unwrap_or_else(|| tr("moderation_unknown_reporter"));
    chat.r#type = ChatType::from_wire_ignoring_case(&facts.string(events::CHAT_TYPE).unwrap_or_default())
        .unwrap_or(ChatType::Direct);
    if chat.r#type == ChatType::Direct {
        chat.participant.name = chat.title.clone();
    }
    chat
}

fn transcript_host(value: &RiseStoredReportCase) -> ChatInfo {
    let mut chat = synthetic_chat(value);
    chat.r#type = ChatType::Group;
    chat
}

fn adapt(message: &RiseStoredCaseContextMessage, chat_id: &str) -> GetMessageMessage {
    let mut result = GetMessageMessage::new(message.message_id.clone());
    result.chat_id = chat_id.to_owned();
    result.sender_id = message.sender_id.clone();
    result.sender_name =
        message.sender.as_ref().map(|sender| sender.display_name()).unwrap_or_else(|| message.sender_id.clone());
    result.sender_tag = message.sender.as_ref().and_then(|sender| sender.tag.clone()).unwrap_or_default();
    result.sender_logo = message.sender.as_ref().and_then(|sender| sender.logo.clone());
    result.sender_is_official = message.sender.as_ref().is_some_and(|sender| sender.is_official);
    result.created_at = message.created_at_ms as f64 / 1000.0;

    let content_type = MessageContentType::from_raw_value(&message.content_type);
    if let Some(media) = message.media_item() {
        result.content_type = content_type;
        result.media_items = vec![media];
        result.text = message.content.clone();
    } else {
        result.text = if message.content.is_empty() {
            format!("[{}]", content_type.raw_value())
        } else {
            message.content.clone()
        };
        result.content_type = MessageContentType::Text;
    }
    result
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

impl RenderOnce for CaseContextPage {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let nav_store = env::<NavigationStoreKey>(cx);
        let state = window.use_keyed_state(self.element_id("state"), cx, |_, cx| CaseContextPageState::new(cx));
        let resolved_id = self.report_id.parse::<i64>().ok();
        let presentation = RiseModerationPresentation::shared(cx);
        let value = resolved_id.and_then(|id| presentation.read(cx).cases().get(&id).cloned());

        let mut shell = ListShellUi::new()
            .center_slot_self_styled(true)
            .shows_back_button(true)
            .on_back(move |_, cx| {
                if let Some(nav_store) = &nav_store {
                    nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
                }
            })
            .fills_viewport_background(false);
        if let Some(value) = &value {
            shell = shell.center_slot(self.header(value, cx));
        }

        shell
            .content(CaseContextBody { report_id: self.report_id.clone(), value, state })
            .on_appear(self.element_id("task"), move |_, cx| {
                if let Some(id) = resolved_id {
                    ModerationInteractionsStore::shared(cx).update(cx, |store, cx| {
                        store.open_case(id, cx);
                        store.case_screen_opened(id);
                    });
                }
            })
            .on_disappear(self.element_id("closed"), move |_, cx| {
                if let Some(id) = resolved_id {
                    ModerationInteractionsStore::shared(cx).update(cx, |store, _| store.case_screen_closed(id));
                }
            })
    }
}

#[derive(IntoElement)]
struct CaseContextBody {
    report_id: String,
    value: Option<RiseStoredReportCase>,
    state: Entity<CaseContextPageState>,
}

impl CaseContextBody {
    fn element_id(&self, name: impl Into<SharedString>) -> ElementId {
        context_element_id(&self.report_id, name)
    }

    fn transcript(&self, value: &RiseStoredReportCase, cx: &mut App) -> AnyElement {
        let chat = transcript_host(value);
        let messages: Vec<GetMessageMessage> = value.context.iter().map(|message| adapt(message, &chat.id)).collect();
        let flags = ChatMessageGrouping::flags(&messages, &chat, cx);
        let reported_id = value.context.iter().find(|message| message.is_reported).map(|message| message.message_id.clone());
        let highlight = rise_ui::theme(cx).semantic.error_100.opacity(0.16);
        let scroll = self.state.read(cx).scroll.clone();

        let rows = messages.iter().enumerate().map(|(index, message)| {
            let (is_first, is_last) = flags.get(index).map_or((true, true), |group| (group.is_first, group.is_last));
            let is_reported = reported_id.as_deref() == Some(message.id.as_str());
            div()
                .w_full()
                .flex()
                .flex_col()
                .items_start()
                .when_some(day_separator(&messages, index), |column, label| {
                    column.child(div().w_full().pt(px(6.0)).pb(px(2.0)).child(MessageSystemPill::new(label)))
                })
                .child(
                    div()
                        .relative()
                        .w_full()
                        .pt(px(if is_first {
                            MessageSizes::list_first_in_group_top_padding()
                        } else {
                            MessageSizes::list_continuation_top_padding()
                        }))
                        .pb(px(if is_last { MessageSizes::list_last_in_group_bottom_padding() } else { 0.0 }))
                        .bg(if is_reported { highlight } else { transparent_black() })
                        .child(
                            MessageRow::new(
                                message.clone(),
                                chat.clone(),
                                false,
                                is_first,
                                is_last,
                                is_last,
                            )
                            .id(self.element_id(format!("message-{}", message.id)))
                            .draws_inline_avatar(is_last)
                            .allows_reply(false)
                            .interactions_enabled(false)
                            .allows_composer_actions(false),
                        )
                        .when(is_reported, |row| row.child(self.center_reported())),
                )
                .into_any_element()
        });

        let transcript = div()
            .w_full()
            .flex()
            .flex_col()
            .items_start()
            .gap(px(MessageSizes::list_row_spacing()))
            .pt(px(12.0))
            .pb(px(ModerationMetrics::BOTTOM_INSET))
            .children(rows.collect::<Vec<_>>());

        env_scope::<ChatGalleryMessagesKey>(Some(Arc::new(Vec::new())))
            .child(
                env_scope::<ChatGalleryPaginatesKey>(false).child(
                    scroll_view(self.element_id("scroll"))
                        .scroll_handle(scroll)
                        .shows_indicators(false)
                        .list_shell_scroll_under_chrome(cx)
                        .child(transcript),
                ),
            )
            .into_any_element()
    }

    fn center_reported(&self) -> impl IntoElement {
        let state = self.state.clone();
        measure(self.element_id("reported-frame"), move |bounds, _window, cx| {
            let (did_center, scroll) = {
                let current = state.read(cx);
                (current.did_center_reported, current.scroll.clone())
            };
            if did_center {
                return;
            }
            let viewport = scroll.bounds();
            let within_content = bounds.origin.y - viewport.origin.y - scroll.offset().y;
            let target = within_content + bounds.size.height * 0.5 - viewport.size.height * 0.5;
            scroll.set_offset(point(px(0.0), -target.max(px(0.0)).min(scroll.max_offset().y)));
            state.update(cx, |state, cx| {
                state.did_center_reported = true;
                cx.notify();
            });
        })
    }
}

impl RenderOnce for CaseContextBody {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let body = match &self.value {
            Some(value) if !value.context.is_empty() => self.transcript(value, cx),
            None => scroll_view(self.element_id("skeleton"))
                .scroll_disabled(true)
                .list_shell_scroll_under_chrome(cx)
                .child(ModerationCaseSkeleton::new())
                .into_any_element(),
            Some(_) => div()
                .size_full()
                .list_shell_scroll_under_chrome(cx)
                .child(ModerationEmptyState::new("text.bubble", tr("case_context_section"), theme))
                .into_any_element(),
        };
        zstack(Alignment::Center)
            .relative()
            .size_full()
            .child(div().absolute().inset_0().child(ChatDecorativeBackdrop::new().ignores_safe_area(true)))
            .child(body)
    }
}
