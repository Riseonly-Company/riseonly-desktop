#![cfg(any(debug_assertions, feature = "sai_debug_tools"))]

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::LazyLock;

use gpui::{App, AsyncApp, Context, Entity, IntoElement, Render, Styled, Subscription, Window, div, px};
use rise_core::foundation::foundation_calendar::Calendar;
use rise_core::foundation::foundation_date::Date;
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::lifecycle::on_change;

use crate::core::lib::canon_format::RiseFormat;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_debug_registration::{
    BottomSheetDebugRegistration, BottomSheetDebugRegistrationMode, BottomSheetDebugRegistrationModule,
};
use crate::modules::chat::components::chat::calendar::chat_calendar_sheet::ChatCalendarSheet;
use crate::modules::chat::components::chat::join_requests::join_chat_sheet::JoinChatSheetContent;
use crate::modules::chat::engine::messages::rise_chat_calendar::{
    RiseChatCalendarDay, RiseChatCalendarMonth, RiseChatCalendarPage, RiseChatCalendarWindow,
};
use crate::modules::chat::pages::chats::chat::chat_profile::chat_profile_more_sheets::AutoDeleteSheet;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{
    ChatJoinPreview, ChatType, JoinLinkSheetPhase, JoinLinkSheetState,
};
use crate::modules::chat::stores::message::message_services::message_calendar_services::{
    MessageChatCalendarPresentation, MessageChatCalendarPresentationLoadState,
};
use crate::modules::chat::widgets::bottomsheets::chat_moderation_sheet::chat_moderation_sheet::{
    ChatModerationSheet, ChatModerationTarget,
};
use crate::modules::chat::widgets::bottomsheets::create_chat_sheet::create_chat_sheet_state::CreateChatSheetState;

/// a callback without a window closes the preview after the current update when the window
/// can be borrowed again
fn close_deferred(open: &Binding<bool>, cx: &mut App) {
    let open = open.clone();
    cx.defer(move |cx| {
        let Some(window) = cx.active_window() else {
            return;
        };
        let _ = window.update(cx, |_, window, cx| open.set(false, window, cx));
    });
}

pub enum ChatBottomSheetPreviews {}

impl ChatBottomSheetPreviews {
    pub fn entries() -> Vec<BottomSheetDebugRegistration> {
        let join: [(&'static str, &'static str, ChatType, bool, &'static str); 8] = [
            ("group", "join_action_join", ChatType::Group, false, ""),
            ("channel", "join_action_join_channel", ChatType::Channel, false, ""),
            ("approval", "join_action_request", ChatType::Group, true, ""),
            ("pending", "join_request_pending", ChatType::Group, true, "join_request_pending"),
            ("expired", "join_link_expired", ChatType::Group, false, "link_expired"),
            ("limit", "join_link_limit_reached", ChatType::Group, false, "link_usage_limit_reached"),
            ("revoked", "join_link_invalid", ChatType::Group, false, "link_revoked"),
            ("banned", "join_banned", ChatType::Group, false, "user_banned"),
        ];
        let mut entries: Vec<BottomSheetDebugRegistration> = join
            .into_iter()
            .map(|(id, title, r#type, approval, status)| {
                let module = if id == "limit" {
                    BottomSheetDebugRegistrationModule::Limits
                } else {
                    BottomSheetDebugRegistrationModule::Chats
                };
                BottomSheetDebugRegistration::sheet(
                    format!("chats.join.{id}"),
                    module,
                    title,
                    "JoinChatSheetContent",
                    vec![0.48, 0.75],
                    None,
                    move |open: Binding<bool>, _, _| {
                        let mut state = JoinLinkSheetState::new("debug-preview".to_owned());
                        state.phase = JoinLinkSheetPhase::Preview(ChatJoinPreview {
                            chat_id: "debug-preview-group".to_owned(),
                            title: "RISE Design Club".to_owned(),
                            logo_url: None,
                            description: Some("A place for ideas, inspiration and conversations.".to_owned()),
                            member_count: 128,
                            r#type,
                            is_verified: true,
                            is_member: false,
                            can_join: status.is_empty(),
                            requires_approval: approval,
                            join_status: status.to_owned(),
                            voice: None,
                        });
                        let retry = open.clone();
                        let submit = open.clone();
                        let request = open;
                        JoinChatSheetContent::new(
                            state,
                            Rc::new(move |cx: &mut App| close_deferred(&retry, cx)),
                            Rc::new(move |_: String, _: String, cx: &mut App| close_deferred(&submit, cx)),
                            Rc::new(move |_: String, _: String, cx: &mut App| close_deferred(&request, cx)),
                        )
                    },
                )
            })
            .collect();
        entries.push(BottomSheetDebugRegistration::sheet(
            "chats.join.loading",
            BottomSheetDebugRegistrationModule::Chats,
            "join_preview_loading",
            "JoinChatSheetContent",
            vec![0.4],
            None,
            move |open: Binding<bool>, _, _| {
                JoinChatSheetContent::new(
                    JoinLinkSheetState::new("debug-preview".to_owned()),
                    Rc::new(move |cx: &mut App| close_deferred(&open, cx)),
                    Rc::new(|_: String, _: String, _: &mut App| {}),
                    Rc::new(|_: String, _: String, _: &mut App| {}),
                )
            },
        ));
        entries.push(BottomSheetDebugRegistration::new(
            "messages.moderation",
            BottomSheetDebugRegistrationModule::Messages,
            "moderation_title_one",
            "ChatModerationSheet",
            move |open: Binding<bool>, _, _| {
                let read = open.clone();
                let write = open.clone();
                let confirm = open;
                ChatModerationSheet::new(
                    "messages.moderation.sheet",
                    Binding::new(
                        move |cx: &App| {
                            read.get(cx).then(|| ChatModerationTarget {
                                chat_id: String::new(),
                                message_id: "debug-message".to_owned(),
                                sender_id: "debug-user".to_owned(),
                                sender_name: "Alex Morgan".to_owned(),
                                is_channel: false,
                            })
                        },
                        move |value: Option<ChatModerationTarget>, window: &mut Window, cx: &mut App| {
                            if value.is_none() {
                                write.set(false, window, cx);
                            }
                        },
                    ),
                    move |_, _, cx: &mut App| close_deferred(&confirm, cx),
                )
            },
        ));
        entries.push(BottomSheetDebugRegistration::sheet(
            "messages.calendar",
            BottomSheetDebugRegistrationModule::Messages,
            "chat_calendar_title",
            "ChatCalendarSheet",
            vec![1.0],
            None,
            move |open: Binding<bool>, _, _| {
                let select = open.clone();
                ChatCalendarSheet::new(
                    "messages.calendar.sheet",
                    open,
                    move |_, window: &mut Window, cx: &mut App| select.set(false, window, cx),
                    |_: &mut Window, _: &mut App| {},
                    |_: &mut Window, _: &mut App| {},
                )
                .presentation_override(DebugChatCalendarFixture::presentation())
            },
        ));
        let app: Rc<RefCell<Option<AsyncApp>>> = Rc::new(RefCell::new(None));
        let content_app = app.clone();
        entries.push(
            BottomSheetDebugRegistration::new(
                "chats.create",
                BottomSheetDebugRegistrationModule::Chats,
                "create_chat_sheet_title",
                "CreateChatSheet",
                move |open: Binding<bool>, window: &mut Window, cx: &mut App| {
                    *content_app.borrow_mut() = Some(cx.to_async());
                    window.use_keyed_state("chats.create.launcher", cx, move |_, cx| {
                        DebugCreateChatLauncher::new(open, cx)
                    })
                },
            )
            .with_mode(BottomSheetDebugRegistrationMode::Live)
            .on_cancel(move || {
                let Some(app) = app.borrow().clone() else {
                    return;
                };
                app.spawn(async move |cx| {
                    cx.update(|cx| {
                        CreateChatSheetState::shared(cx).update(cx, |state, cx| state.close(cx));
                    });
                })
                .detach();
            }),
        );
        entries.push(BottomSheetDebugRegistration::native(
            "messages.auto_delete",
            BottomSheetDebugRegistrationModule::Messages,
            "chat_autodelete_title",
            "AutoDeleteSheet",
            move |open: Binding<bool>, _, _| {
                AutoDeleteSheet::new("debug-chat", 604_800)
                    .on_submit(move |_, cx: &mut App| close_deferred(&open, cx))
            },
        ));
        entries
    }
}

enum DebugChatCalendarFixture {}

impl DebugChatCalendarFixture {
    fn presentation() -> MessageChatCalendarPresentation {
        static PRESENTATION: LazyLock<MessageChatCalendarPresentation> =
            LazyLock::new(DebugChatCalendarFixture::make_presentation);
        PRESENTATION.clone()
    }

    fn make_presentation() -> MessageChatCalendarPresentation {
        let calendar = Calendar::current();
        let locale = RiseFormat::app_locale();
        let now = Date::now();
        let mut value = MessageChatCalendarPresentation::default();
        value.load_state = MessageChatCalendarPresentationLoadState::Loaded;
        value.weekday_symbols = RiseChatCalendarMonth::weekday_symbols(&calendar, &locale);
        let Some(window) = RiseChatCalendarWindow::month(&now, &calendar) else {
            return value;
        };
        let today = RiseChatCalendarWindow::milliseconds(&calendar.start_of_day(&now));
        let days: Vec<RiseChatCalendarDay> = window
            .day_starts
            .iter()
            .enumerate()
            .filter_map(|(index, &ms)| {
                if ms > today || index % 3 == 1 {
                    return None;
                }
                Some(RiseChatCalendarDay {
                    day_start_ms: ms,
                    first_message_id: format!("debug-day-{index}"),
                    first_message_at_ms: ms,
                    preview_url: None,
                    preview_is_video: false,
                })
            })
            .collect();
        let title = RiseFormat::date_formatter("LLLL yyyy", &locale, &calendar, None).string_from(&now);
        value.months = vec![RiseChatCalendarMonth::make(
            &window,
            &RiseChatCalendarPage { days, has_older: false },
            &calendar,
            title,
            &now,
        )];
        value
    }
}

struct DebugCreateChatLauncher {
    is_presented: Binding<bool>,
    state: Entity<CreateChatSheetState>,
    _state_observer: Subscription,
}

impl DebugCreateChatLauncher {
    fn new(is_presented: Binding<bool>, cx: &mut Context<Self>) -> Self {
        let state = CreateChatSheetState::shared(cx);
        let state_observer = cx.observe(&state, |_, _, cx| cx.notify());
        Self { is_presented, state, _state_observer: state_observer }
    }
}

impl Render for DebugCreateChatLauncher {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.clone();
        on_change(
            "chats.create.launcher.open",
            self.is_presented.get(cx),
            true,
            window,
            cx,
            move |_, open: &bool, _, cx| {
                if *open && !state.read(cx).is_open() {
                    state.update(cx, |state, cx| state.open(cx));
                }
            },
        );
        let is_open = self.state.read(cx).is_open();
        let is_presented = self.is_presented.clone();
        on_change(
            "chats.create.launcher.state",
            is_open,
            false,
            window,
            cx,
            move |_, open: &bool, window, cx| {
                if !*open {
                    is_presented.set(false, window, cx);
                }
            },
        );
        div().w(px(0.0)).h(px(0.0))
    }
}
