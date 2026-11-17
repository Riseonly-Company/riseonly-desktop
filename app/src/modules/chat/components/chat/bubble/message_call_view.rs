use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, AsyncApp, ElementId, FontWeight, Hsla, InteractiveElement, IntoElement, ParentElement, RenderOnce, SharedString,
    StatefulInteractiveElement, Styled, TextRun, Window, WindowTextSystem, div, px,
};
use rise_chat::ChatType;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::date_format::date_formatter::{DateFormatter, Style};
use rise_i18n::{AppDateFormatter, tr};
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::IconUi;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::font::Font;
use rise_ui::swiftui::prelude::*;

use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::bubble::bubble_tap_router::BubbleTapRouterModifiers;
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageCallInfo, MessageCallTerminalState,
};
use crate::modules::rtc::stores::rtc::rtc_interactions::rtc_interactions::RTCInteractionsStore;
use crate::modules::rtc::system::rise_rtc_call_system_coordinator::RiseRTCCallStartRefusal;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MessageCallDirection {
    Outgoing,
    Incoming,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MessageCallPresentationOutcome {
    Completed,
    Cancelled,
    Declined,
    Missed,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MessageCallAccent {
    Success,
    Error,
    Neutral,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MessageCallPresentation {
    pub title: String,
    pub detail: String,
    pub direction: MessageCallDirection,
    pub outcome: MessageCallPresentationOutcome,
    pub accent: MessageCallAccent,
    pub is_video: bool,
}

impl MessageCallPresentation {
    pub fn direction_system_image(&self) -> &'static str {
        match self.direction {
            MessageCallDirection::Outgoing => "arrow.up.right",
            MessageCallDirection::Incoming => "arrow.down.left",
        }
    }

    pub fn media_system_image(&self) -> &'static str {
        if self.is_video { "video.fill" } else { "phone.fill" }
    }
}

thread_local! {
    static TIME_FORMATTER: DateFormatter = {
        let mut formatter = AppDateFormatter::make(None, None, None);
        formatter.set_date_style(Style::None);
        formatter.set_time_style(Style::Short);
        formatter
    };
    static CALLS_IN_FLIGHT: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

pub enum MessageCallPresentationFactory {}

impl MessageCallPresentationFactory {
    pub fn make(message: &GetMessageMessage, fallback_is_own: bool, cx: &mut App) -> Option<MessageCallPresentation> {
        let call = message.call_info.as_ref()?;
        let direction = Self::resolved_direction(call, fallback_is_own, cx);
        let outcome = Self::resolved_outcome(call, direction);
        let timestamp = if message.created_at > 0.0 { message.created_at } else { call.ended_at_ms as f64 / 1_000.0 };
        let time = TIME_FORMATTER
            .with(|formatter| formatter.string_from(&Date::from_time_interval_since_1970(timestamp)));
        let detail = match Self::format_duration(call.resolved_duration_ms()) {
            Some(duration) => format!("{time} \u{b7} {duration}"),
            None => time,
        };
        let is_video = call.media_mode.is_video();

        Some(MessageCallPresentation {
            title: Self::title(outcome, direction, is_video),
            detail,
            direction,
            outcome,
            accent: Self::accent(outcome),
            is_video,
        })
    }

    pub fn preview_title(message: &GetMessageMessage, cx: &mut App) -> String {
        let auth = AuthServicesStore::shared(cx);
        let fallback_is_own = auth.read(cx).is_current_user(&message.sender_id);
        Self::make(message, fallback_is_own, cx)
            .map(|presentation| presentation.title)
            .unwrap_or_else(|| tr("chat_call_generic"))
    }

    fn resolved_direction(call: &MessageCallInfo, fallback_is_own: bool, cx: &mut App) -> MessageCallDirection {
        let auth = AuthServicesStore::shared(cx);
        let current_user_id = auth.read(cx).effective_user_id_for_wire();
        let current_user_id = current_user_id.as_deref().map(trimming_whitespaces_and_newlines);
        if let Some(current_user_id) = current_user_id.filter(|id| !id.is_empty()) {
            if current_user_id == call.caller_user_id {
                return MessageCallDirection::Outgoing;
            }
            if current_user_id == call.callee_user_id {
                return MessageCallDirection::Incoming;
            }
        }
        if fallback_is_own { MessageCallDirection::Outgoing } else { MessageCallDirection::Incoming }
    }

    fn resolved_outcome(call: &MessageCallInfo, direction: MessageCallDirection) -> MessageCallPresentationOutcome {
        let dropped_outcome = if direction == MessageCallDirection::Outgoing {
            MessageCallPresentationOutcome::Cancelled
        } else {
            MessageCallPresentationOutcome::Missed
        };
        match call.terminal_state {
            MessageCallTerminalState::Ended => MessageCallPresentationOutcome::Completed,
            MessageCallTerminalState::Cancelled | MessageCallTerminalState::TimedOut => dropped_outcome,
            MessageCallTerminalState::Declined => MessageCallPresentationOutcome::Declined,
            MessageCallTerminalState::Unknown(_) => MessageCallPresentationOutcome::Failed,
        }
    }

    fn title(outcome: MessageCallPresentationOutcome, direction: MessageCallDirection, is_video: bool) -> String {
        match outcome {
            MessageCallPresentationOutcome::Completed => {
                if is_video {
                    return tr("chat_call_video");
                }
                if direction == MessageCallDirection::Outgoing {
                    tr("chat_call_outgoing")
                } else {
                    tr("chat_call_incoming")
                }
            }
            MessageCallPresentationOutcome::Cancelled => tr("chat_call_cancelled"),
            MessageCallPresentationOutcome::Declined => tr("chat_call_declined"),
            MessageCallPresentationOutcome::Missed | MessageCallPresentationOutcome::Failed => tr("chat_call_missed"),
        }
    }

    fn accent(outcome: MessageCallPresentationOutcome) -> MessageCallAccent {
        match outcome {
            MessageCallPresentationOutcome::Completed => MessageCallAccent::Success,
            MessageCallPresentationOutcome::Cancelled
            | MessageCallPresentationOutcome::Declined
            | MessageCallPresentationOutcome::Missed
            | MessageCallPresentationOutcome::Failed => MessageCallAccent::Error,
        }
    }

    fn format_duration(milliseconds: i64) -> Option<String> {
        if milliseconds <= 0 {
            return None;
        }
        let total_seconds = milliseconds / 1_000;
        let seconds = total_seconds % 60;
        let total_minutes = total_seconds / 60;
        if total_minutes < 60 {
            return Some(format!("{total_minutes}:{seconds:02}"));
        }
        Some(format!("{}:{:02}:{seconds:02}", total_minutes / 60, total_minutes % 60))
    }
}

pub enum MessageCallBubbleGeometry {}

impl MessageCallBubbleGeometry {
    pub const PREFERRED_BUBBLE_WIDTH: f32 = 258.0;
    pub const BUBBLE_HEIGHT: f32 = 70.0;
    pub const TITLE_FONT_SIZE: f32 = 17.0;
    pub const TITLE_FONT_WEIGHT: FontWeight = FontWeight::SEMIBOLD;
    pub const DETAIL_FONT_SIZE: f32 = 13.0;
    pub const DETAIL_FONT_WEIGHT: FontWeight = FontWeight::NORMAL;

    /// grows past the preferred width until title and detail fit, a fixed width cut long titles short
    pub fn bubble_width(
        viewport_width: f32,
        chat: &ChatInfo,
        is_own: bool,
        presentation: &MessageCallPresentation,
        cx: &App,
    ) -> f32 {
        Self::PREFERRED_BUBBLE_WIDTH
            .max(Self::fitting_width(presentation, cx))
            .min(MessageSizes::available_bubble_width(viewport_width, chat, is_own))
    }

    pub fn content_width(
        viewport_width: f32,
        chat: &ChatInfo,
        is_own: bool,
        presentation: &MessageCallPresentation,
        cx: &App,
    ) -> f32 {
        (Self::bubble_width(viewport_width, chat, is_own, presentation, cx) - MessageSizes::bubble_tail_inset()).max(1.0)
    }

    fn fitting_width(presentation: &MessageCallPresentation, cx: &App) -> f32 {
        let layout = WindowTextSystem::new(cx.text_system().clone());
        let title = Self::text_width(&presentation.title, Self::TITLE_FONT_SIZE, Self::TITLE_FONT_WEIGHT, &layout);
        let detail = Self::text_width(&presentation.detail, Self::DETAIL_FONT_SIZE, Self::DETAIL_FONT_WEIGHT, &layout);
        let content = title.max(17.0 + detail) + 13.0 + 28.0 + 34.0 + 12.0;
        content.ceil() + MessageSizes::bubble_tail_inset()
    }

    fn text_width(text: &str, font_size: f32, weight: FontWeight, layout: &WindowTextSystem) -> f32 {
        let font = Font::system_weight(font_size, weight);
        let run = TextRun {
            len: text.len(),
            font: gpui::Font {
                family: font.family(),
                features: font.features(),
                fallbacks: None,
                weight,
                style: font.style(),
            },
            color: Hsla::default(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        f32::from(layout.layout_line(text, px(font_size), &[run], None).width)
    }

    pub fn row_height(is_first_in_group: bool, is_last_in_group: bool) -> f32 {
        let top = if is_first_in_group {
            MessageSizes::list_first_in_group_top_padding()
        } else {
            MessageSizes::list_continuation_top_padding()
        };
        let bottom = if is_last_in_group { MessageSizes::list_last_in_group_bottom_padding() } else { 0.0 };
        (top + Self::BUBBLE_HEIGHT + bottom).ceil()
    }
}

#[derive(IntoElement)]
pub struct MessageCallView {
    id: ElementId,
    message: Rc<GetMessageMessage>,
    chat: Rc<ChatInfo>,
    is_own: bool,
    presentation: MessageCallPresentation,
    viewport_width: f32,
}

impl MessageCallView {
    pub fn new(
        id: impl Into<ElementId>,
        message: impl Into<Rc<GetMessageMessage>>,
        chat: impl Into<Rc<ChatInfo>>,
        is_own: bool,
        presentation: MessageCallPresentation,
        viewport_width: f32,
    ) -> Self {
        Self { id: id.into(), message: message.into(), chat: chat.into(), is_own, presentation, viewport_width }
    }

    fn accent_color(&self, theme: &AppTheme) -> Hsla {
        if self.is_own {
            return theme.content_on(theme.primary._100);
        }
        match self.presentation.accent {
            MessageCallAccent::Success => theme.semantic.success_100,
            MessageCallAccent::Error => theme.semantic.error_100,
            MessageCallAccent::Neutral => theme.primary._100,
        }
    }
}

impl RenderOnce for MessageCallView {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let content_width = MessageCallBubbleGeometry::content_width(
            self.viewport_width,
            &self.chat,
            self.is_own,
            &self.presentation,
            cx,
        );
        let label = format!("{}, {}", self.presentation.title, self.presentation.detail);
        let accent = self.accent_color(&theme);
        let message = self.message.clone();
        let chat = self.chat.clone();

        hstack(10.0)
            .flex_none()
            .pl(px(13.0))
            .pr(px(12.0))
            .w(px(content_width))
            .h(px(MessageCallBubbleGeometry::BUBBLE_HEIGHT))
            .cursor_pointer()
            .child(
                vstack(5.0)
                    .align(HorizontalAlignment::Leading)
                    .flex_shrink(1.0)
                    .min_w_0()
                    .child(
                        MainText::new(self.presentation.title.clone())
                            .px(MessageCallBubbleGeometry::TITLE_FONT_SIZE)
                            .font_weight(MessageCallBubbleGeometry::TITLE_FONT_WEIGHT)
                            .color(if self.is_own { theme.content_on(theme.primary._100) } else { theme.text.primary })
                            .number_of_lines(1),
                    )
                    .child(
                        hstack(4.0)
                            .min_w_0()
                            .children(IconUi::sized(self.presentation.direction_system_image(), px(13.0), accent, false))
                            .child(
                                MainText::new(self.presentation.detail.clone())
                                    .px(MessageCallBubbleGeometry::DETAIL_FONT_SIZE)
                                    .color(if self.is_own { theme.content_on(theme.primary._100).opacity(0.7) } else { theme.text.secondary })
                                    .number_of_lines(1),
                            ),
                    ),
            )
            .child(spacer_min_w(8.0))
            .child(
                div().flex_none().flex().items_center().justify_center().frame_size(34.0, 38.0).children(IconUi::sized(
                    self.presentation.media_system_image(),
                    px(27.0),
                    if self.is_own { theme.content_on(theme.primary._100) } else { theme.primary._100 },
                    true,
                )),
            )
            .id(ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static("root")))
            .aria_label(label)
            .bubble_tap_target_priority(self.id, move |_: &mut Window, cx: &mut App| {
                MessageCallActionRouter::call_again(&message, &chat, cx);
            })
    }
}

struct MessageCallInFlightGuard(String);

impl Drop for MessageCallInFlightGuard {
    fn drop(&mut self) {
        CALLS_IN_FLIGHT.with(|calls| {
            calls.borrow_mut().remove(&self.0);
        });
    }
}

pub enum MessageCallActionRouter {}

impl MessageCallActionRouter {
    pub fn call_again(message: &GetMessageMessage, chat: &ChatInfo, cx: &mut App) {
        if chat.r#type != ChatType::Direct {
            return;
        }
        let Some(call) = message.call_info.as_ref() else {
            return;
        };
        let chat_id = trimming_whitespaces_and_newlines(&chat.id).to_owned();
        let participant_id = trimming_whitespaces_and_newlines(&chat.participant.id).to_owned();
        if chat_id.is_empty() || participant_id.is_empty() {
            return;
        }
        if chat.participant.is_deleted {
            Notifier::send(
                tr("rtc_call_failed"),
                NotifierSendOptions {
                    r#type: NotifierType::Error,
                    position: NotifierPosition::Bottom,
                    ..Default::default()
                },
                cx,
            );
            return;
        }
        if !CALLS_IN_FLIGHT.with(|calls| calls.borrow_mut().insert(chat_id.clone())) {
            return;
        }
        let guard = MessageCallInFlightGuard(chat_id.clone());

        let participant_name = trimming_whitespaces_and_newlines(&chat.participant.name);
        let display_name = if participant_name.is_empty() { chat.direct_peer_profile_name() } else { participant_name.to_owned() };
        let avatar_url = if chat.participant.more.logo.is_empty() {
            chat.logo_url.clone()
        } else {
            Some(chat.participant.more.logo.clone())
        };
        let has_video = call.media_mode.is_video();
        let request = RTCInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.request_system_outgoing_call(&participant_id, &chat_id, display_name, avatar_url, has_video, cx)
        });
        cx.spawn(async move |cx: &mut AsyncApp| {
            let _guard = guard;
            if let Err(error) = request.await {
                cx.update(|cx| {
                    Notifier::send(
                        RiseRTCCallStartRefusal::text_or_failed(&error),
                        NotifierSendOptions {
                            r#type: NotifierType::Error,
                            position: NotifierPosition::Bottom,
                            ..Default::default()
                        },
                        cx,
                    );
                });
            }
        })
        .detach();
    }
}
