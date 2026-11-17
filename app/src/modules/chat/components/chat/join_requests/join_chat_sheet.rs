use std::rc::Rc;

use gpui::{
    AnyElement, App, Context, FocusHandle, FontWeight, SharedString, Subscription, Window, black,
};
use rise_catalog::ChatInviteJoinStatus;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::{plural_word, tr, tr_plural};
use rise_theme::AppTheme;
use rise_theme::theme::theme_store::ThemeStore;
use rise_ui::IconUi;
use rise_ui::bottom_sheet_ui::modal_sheet_surface::{
    ModalSheetFrame, ModalSheetMetrics, modal_sheet_centering, modal_sheet_chrome, modal_sheet_outline,
};
use rise_ui::layer_keys::{LayerCancel, LayerKeysExt};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::ease_out;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_types::RiseRTCRoomTarget;
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh::SaiNavigationBridge;
use crate::core::push_notifications::push_deep_link_store::PushDeepLink;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierIcon, NotifierPosition, NotifierSendOptions, NotifierType,
};
use crate::modules::chat::stores::chats::chat_actions::chat_types::{
    ChatJoinPreview, ChatVoiceJoinPreview, JoinLinkSheetPhase, JoinLinkSheetState,
};
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;

type OpenCall = Rc<dyn Fn(String, RiseRTCRoomTarget, &mut App)>;
type OpenTopic = Rc<dyn Fn(String, String, &mut App)>;

pub struct JoinChatSheetHost {
    on_open_chat: Rc<dyn Fn(String, &mut App)>,
    on_open_call: OpenCall,
    on_open_topic: OpenTopic,
    presented: Animated<f32>,
    shown: Option<JoinLinkSheetState>,
    focus: FocusHandle,
    previous_focus: Option<FocusHandle>,
    _subscriptions: Vec<Subscription>,
}

impl JoinChatSheetHost {
    pub fn new(
        on_open_chat: impl Fn(String, &mut App) + 'static,
        on_open_call: impl Fn(String, RiseRTCRoomTarget, &mut App) + 'static,
        on_open_topic: impl Fn(String, String, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) -> Self {
        let services = ChatServicesStore::shared(cx);
        Self {
            on_open_chat: Rc::new(on_open_chat),
            on_open_call: Rc::new(on_open_call),
            on_open_topic: Rc::new(on_open_topic),
            presented: Animated::new(0.0),
            shown: None,
            focus: cx.focus_handle(),
            previous_focus: None,
            _subscriptions: vec![cx.observe(&services, |host, services, cx| {
                if services.read(cx).join_link_sheet.is_some() || host.shown.is_some() {
                    cx.notify();
                }
            })],
        }
    }

    fn is_presentable(cx: &mut App) -> bool {
        ChatServicesStore::shared(cx).read(cx).join_link_sheet.is_some()
    }

    fn dismiss(cx: &mut App) {
        ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.dismiss_join_link_sheet(cx));
    }

    fn retry(cx: &mut App) {
        if !Self::is_presentable(cx) {
            return;
        }
        ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.retry_invite_link_preview(cx));
    }

    fn open_group(chat_id: String, on_open_chat: &Rc<dyn Fn(String, &mut App)>, cx: &mut App) {
        Self::dismiss(cx);
        Self::open_chat_if_needed(chat_id, on_open_chat, cx);
    }

    fn open_chat_if_needed(chat_id: String, on_open_chat: &Rc<dyn Fn(String, &mut App)>, cx: &mut App) {
        let route = PushDeepLink::Chat { chat_id: chat_id.clone() };
        let active_screen = SaiNavigationBridge::current_screen(cx);
        if route.matches(active_screen.as_ref()) {
            return;
        }
        on_open_chat(chat_id, cx);
    }

    fn handle_join(
        chat_id: &str,
        token: &str,
        on_open_chat: Rc<dyn Fn(String, &mut App)>,
        on_open_call: OpenCall,
        on_open_topic: OpenTopic,
        cx: &mut App,
    ) {
        if !Self::is_presentable(cx) {
            return;
        }
        let voice = ChatServicesStore::shared(cx)
            .read(cx)
            .join_link_sheet
            .as_ref()
            .and_then(|state| state.preview())
            .and_then(|preview| preview.voice.clone());
        ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.join_via_sheet(
                chat_id,
                token,
                Box::new(move |joined_id: Option<String>, cx: &mut App| {
                    cx.defer(move |cx| {
                        let Some(joined_id) = joined_id else {
                            return;
                        };
                        Self::dismiss(cx);
                        if let Some(voice) = voice {
                            Self::open(voice, joined_id, &on_open_call, &on_open_topic, cx);
                            return;
                        }
                        Self::open_chat_if_needed(joined_id, &on_open_chat, cx);
                    });
                }),
                cx,
            );
        });
    }

    /// a text topic opens over its group topic list, a voice chat enters the call
    fn open(
        voice: ChatVoiceJoinPreview,
        chat_id: String,
        on_open_call: &OpenCall,
        on_open_topic: &OpenTopic,
        cx: &mut App,
    ) {
        if voice.is_text
            && let RiseRTCRoomTarget::Topic { topic_id, .. } = &voice.target
        {
            on_open_topic(chat_id, topic_id.clone(), cx);
            return;
        }
        let target = Self::retarget(voice.target, &chat_id);
        on_open_call(chat_id, target, cx);
    }

    /// joined id is the canonical one, the call keeps its topic
    fn retarget(target: RiseRTCRoomTarget, chat_id: &str) -> RiseRTCRoomTarget {
        match target {
            RiseRTCRoomTarget::Topic { topic_id, .. } => {
                RiseRTCRoomTarget::Topic { chat_id: chat_id.to_owned(), topic_id }
            }
            RiseRTCRoomTarget::Group { .. } => RiseRTCRoomTarget::Group { chat_id: chat_id.to_owned() },
            RiseRTCRoomTarget::DirectCall { .. } => target,
        }
    }

    fn handle_request(chat_id: &str, token: &str, cx: &mut App) {
        if !Self::is_presentable(cx) {
            return;
        }
        ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.request_join_via_sheet(
                chat_id,
                token,
                Box::new(move |ok: bool, cx: &mut App| {
                    cx.defer(move |cx| {
                        if !ok {
                            return;
                        }
                        Self::dismiss(cx);
                        Notifier::send(
                            tr("join_request_sent_title"),
                            NotifierSendOptions {
                                content: Some(tr("join_request_sent_subtitle")),
                                r#type: NotifierType::Success,
                                position: NotifierPosition::Bottom,
                                icon: NotifierIcon::Emoji("\u{2705}".to_owned()),
                                ..NotifierSendOptions::default()
                            },
                            cx,
                        );
                    });
                }),
                cx,
            );
        });
    }
}

impl Render for JoinChatSheetHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let state = ChatServicesStore::shared(cx).read(cx).join_link_sheet.clone();
        let presentable = state.is_some();
        let now = RiseClock::monotonic();
        let this = cx.entity();

        let observer = this.downgrade();
        on_change("join-chat-sheet-presentable", presentable, true, window, cx, move |_, is_on, window, cx| {
            let is_on = *is_on;
            let _ = observer.update(cx, |host, cx| {
                if is_on {
                    host.previous_focus = window.focused(cx);
                    window.focus(&host.focus, cx);
                } else {
                    if let Some(previous) = host.previous_focus.take() {
                        window.focus(&previous, cx);
                    }
                }
                cx.notify();
            });
        });

        let on_open_chat = self.on_open_chat.clone();
        let on_open_call = self.on_open_call.clone();
        let on_open_topic = self.on_open_topic.clone();
        let preview = state.as_ref().and_then(|state| state.preview().cloned());
        on_change("join-chat-sheet-preview", preview, false, window, cx, move |_, preview, _, cx| {
            let Some(preview) =
                preview.as_ref().filter(|preview| preview.is_member && !preview.is_voice_refusal())
            else {
                return;
            };
            Self::dismiss(cx);
            if let Some(voice) = preview.voice.as_ref().filter(|voice| voice.can_join_call || voice.is_text) {
                Self::open(voice.clone(), preview.chat_id.clone(), &on_open_call, &on_open_topic, cx);
                return;
            }
            Self::open_chat_if_needed(preview.chat_id.clone(), &on_open_chat, cx);
            let title_key =
                if preview.is_channel() { "join_already_member_channel" } else { "join_already_member_group" };
            Notifier::send(
                tr(title_key),
                NotifierSendOptions {
                    r#type: NotifierType::Info,
                    position: NotifierPosition::Bottom,
                    ..NotifierSendOptions::default()
                },
                cx,
            );
        });

        let (target, duration) = if presentable {
            (1.0, ModalSheetMetrics::PRESENT_DURATION)
        } else {
            (0.0, ModalSheetMetrics::DISMISS_DURATION)
        };
        self.presented.set_in(target, ease_out(duration), now, cx);
        let fraction = self.presented.frame_value(now, window);
        if state.is_some() {
            self.shown = state;
        } else if !self.presented.is_running(now) {
            self.shown = None;
        }
        let Some(shown) = self.shown.clone() else {
            return div().into_any_element();
        };

        let on_open_chat = self.on_open_chat.clone();
        let on_open_call = self.on_open_call.clone();
        let on_open_topic = self.on_open_topic.clone();
        let group_opener = self.on_open_chat.clone();
        let frame = ModalSheetFrame::for_window(window, cx);
        let content = div()
            .id("join-chat-sheet-body")
            .w_full()
            .max_h(px(frame.content_max_height()))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .items_center()
            .pt(px(8.0))
            .pb(px(14.0))
            .child(JoinChatSheetContent::new(
                shown,
                Rc::new(|cx: &mut App| Self::retry(cx)),
                Rc::new(move |chat_id: String, token: String, cx: &mut App| {
                    Self::handle_join(
                        &chat_id,
                        &token,
                        on_open_chat.clone(),
                        on_open_call.clone(),
                        on_open_topic.clone(),
                        cx,
                    );
                }),
                Rc::new(|chat_id: String, token: String, cx: &mut App| {
                    Self::handle_request(&chat_id, &token, cx);
                }),
            )
            .on_open_group(move |chat_id: String, cx: &mut App| Self::open_group(chat_id, &group_opener, cx)));
        let surface = div()
            .id("join-chat-sheet-panel")
            .relative()
            .w(px(frame.width))
            .when(presentable, |panel| panel.occlude())
            .child(content);
        let surface = modal_sheet_chrome(surface, theme.bg._100, modal_sheet_outline(&theme), fraction);

        let focus = self.focus.clone();
        div()
            .id("join-chat-sheet-host")
            .absolute()
            .inset_0()
            .size_full()
            .when(presentable, |host| {
                host.track_focus(&focus).layer_keys(LayerCancel::dismiss(|_: &mut Window, cx: &mut App| Self::dismiss(cx)))
            })
            .child(
                div()
                    .id("join-chat-sheet-scrim")
                    .absolute()
                    .inset_0()
                    .bg(black().opacity(0.5 * fraction.clamp(0.0, 1.0)))
                    .when(presentable, |scrim| {
                        scrim.occlude().on_click(move |_, _, cx| Self::dismiss(cx))
                    }),
            )
            .child(modal_sheet_centering().child(surface))
            .into_any_element()
    }
}

#[derive(IntoElement)]
pub struct JoinChatSheetContent {
    state: JoinLinkSheetState,
    on_retry: Rc<dyn Fn(&mut App)>,
    on_join: Rc<dyn Fn(String, String, &mut App)>,
    on_request: Rc<dyn Fn(String, String, &mut App)>,
    on_open_group: Rc<dyn Fn(String, &mut App)>,
}

impl JoinChatSheetContent {
    pub fn new(
        state: JoinLinkSheetState,
        on_retry: Rc<dyn Fn(&mut App)>,
        on_join: Rc<dyn Fn(String, String, &mut App)>,
        on_request: Rc<dyn Fn(String, String, &mut App)>,
    ) -> Self {
        Self { state, on_retry, on_join, on_request, on_open_group: Rc::new(|_, _| {}) }
    }

    pub fn on_open_group(mut self, on_open_group: impl Fn(String, &mut App) + 'static) -> Self {
        self.on_open_group = Rc::new(on_open_group);
        self
    }

    fn preview_state(&self, preview: &ChatJoinPreview, theme: &AppTheme) -> AnyElement {
        vstack(14.0)
            .w_full()
            .items_center()
            .child(
                div().pt(px(6.0)).child(
                    UserLogoUi::new("join-chat-sheet-logo")
                        .source(preview.logo_url.clone().map(SharedString::from))
                        .name(SharedString::from(preview.title.clone()))
                        .user_id(SharedString::from(preview.chat_id.clone()))
                        .size(88.0),
                ),
            )
            .child(
                vstack(5.0)
                    .items_center()
                    .child(
                        hstack(6.0)
                            .items_center()
                            .child(
                                div().min_w_0().child(
                                    MainText::new(preview.title.clone())
                                        .px(21.0)
                                        .font_weight(FontWeight::BOLD)
                                        .color(theme.text.primary)
                                        .line_limit(2),
                                ),
                            )
                            .when(preview.is_verified, |title| {
                                title.children(IconUi::sized(
                                    "checkmark.seal.fill",
                                    px(16.0),
                                    theme.primary._100,
                                    true,
                                ))
                            }),
                    )
                    .when(preview.member_count > 0, |column| {
                        column.child(
                            MainText::new(Self::member_count_text(preview)).px(14.0).color(theme.text.secondary),
                        )
                    }),
            )
            .when_some(preview.voice.as_ref().filter(|voice| !voice.details_hidden), |column, voice| {
                column.child(Self::voice_card(voice, theme))
            })
            .when_some(preview.description.clone().filter(|description| !description.is_empty()), |column, text| {
                column.child(
                    div().w_full().px(px(4.0)).child(
                        MainText::new(text)
                            .px(14.0)
                            .tac(TextAlignT::Center)
                            .color(theme.text.secondary)
                            .line_limit(4),
                    ),
                )
            })
            .children(self.action_area(preview, theme))
            .into_any_element()
    }

    fn action_area(&self, preview: &ChatJoinPreview, theme: &AppTheme) -> Vec<AnyElement> {
        let mut views = Vec::new();
        if let Some(error) = self.state.submission_error.clone().filter(|error| !error.is_empty()) {
            views.push(
                div()
                    .pt(px(12.0))
                    .pb(px(6.0))
                    .child(MainText::new(error).px(14.0).tac(TextAlignT::Center).color(theme.semantic.error_100))
                    .into_any_element(),
            );
        }

        if !preview.can_join {
            views.push(
                div()
                    .pt(px(16.0))
                    .pb(px(10.0))
                    .child(
                        MainText::new(Self::reason_text(&preview.join_status))
                            .px(14.0)
                            .tac(TextAlignT::Center)
                            .color(theme.text.secondary),
                    )
                    .into_any_element(),
            );
            if preview.is_voice_refusal() {
                let chat_id = preview.chat_id.clone();
                let on_open_group = self.on_open_group.clone();
                views.push(
                    plain("join-chat-sheet-open-group")
                        .w_full()
                        .h(px(52.0))
                        .on_press(move |_, cx| on_open_group(chat_id.clone(), cx))
                        .child(
                            div()
                                .size_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(15.0))
                                .bg(theme.bg._200)
                                .child(
                                    MainText::new(tr("join_open_group"))
                                        .px(16.0)
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .color(theme.primary._100),
                                ),
                        )
                        .into_any_element(),
                );
            }
            return views;
        }

        let is_submitting = self.state.is_submitting();
        let label = if is_submitting {
            progress_view().tint(theme.content_on(theme.primary._100)).into_any_element()
        } else {
            MainText::new(Self::join_button_title(preview))
                .px(16.0)
                .font_weight(FontWeight::SEMIBOLD)
                .color(theme.content_on(theme.primary._100))
                .into_any_element()
        };
        let chat_id = preview.chat_id.clone();
        let requires_approval = preview.requires_approval;
        let token = self.state.token.clone();
        let on_join = self.on_join.clone();
        let on_request = self.on_request.clone();
        views.push(
            div()
                .w_full()
                .pt(px(6.0))
                .child(
                    plain("join-chat-sheet-action")
                        .w_full()
                        .h(px(52.0))
                        .when(is_submitting, |button| button.opacity(0.85))
                        .disabled(is_submitting)
                        .on_press(move |_, cx| {
                            if requires_approval {
                                on_request(chat_id.clone(), token.clone(), cx);
                            } else {
                                on_join(chat_id.clone(), token.clone(), cx);
                            }
                        })
                        .child(
                            div()
                                .size_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(15.0))
                                .bg(theme.primary._100)
                                .child(label),
                        ),
                )
                .into_any_element(),
        );
        views
    }

    fn loading_state(theme: &AppTheme) -> AnyElement {
        vstack(14.0)
            .w_full()
            .min_h(px(220.0))
            .items_center()
            .justify_center()
            .child(progress_view().control_size(ControlSize::Large).tint(theme.primary._100))
            .child(
                MainText::new(tr("join_preview_loading"))
                    .px(15.0)
                    .tac(TextAlignT::Center)
                    .color(theme.text.secondary),
            )
            .into_any_element()
    }

    fn empty_state(&self, theme: &AppTheme) -> AnyElement {
        self.status_state("bubble.left.and.bubble.right", tr("join_preview_empty"), true, theme)
    }

    fn error_state(&self, message: &str, retryable: bool, theme: &AppTheme) -> AnyElement {
        let icon = if retryable { "exclamationmark.arrow.triangle.2.circlepath" } else { "link" };
        self.status_state(icon, message.to_owned(), retryable, theme)
    }

    fn status_state(&self, icon: &str, message: String, retryable: bool, theme: &AppTheme) -> AnyElement {
        let on_retry = self.on_retry.clone();
        vstack(15.0)
            .w_full()
            .min_h(px(220.0))
            .items_center()
            .justify_center()
            .children(IconUi::sized(icon, px(38.0), theme.text.secondary, false))
            .child(MainText::new(message).px(15.0).tac(TextAlignT::Center).color(theme.text.secondary))
            .when(retryable, |column| {
                column.child(
                    plain("join-chat-sheet-retry").w_full().on_press(move |_, cx| on_retry(cx)).child(
                        div()
                            .w_full()
                            .h(px(48.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(theme.primary._100)
                            .rounded_full()
                            .child(
                                MainText::new(tr("common_retry"))
                                    .px(15.0)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .color(theme.content_on(theme.primary._100)),
                            ),
                    ),
                )
            })
            .into_any_element()
    }

    fn voice_card(voice: &ChatVoiceJoinPreview, theme: &AppTheme) -> AnyElement {
        let accent = if voice.live { theme.semantic.success_100 } else { theme.text.secondary };
        let title = voice
            .topic_title
            .clone()
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| tr(if voice.is_text { "chat_topics_topic" } else { "rtc_group_voice" }));
        let status = if voice.is_text {
            tr("chat_topics_topic")
        } else if voice.live {
            tr_plural("voice_invite_live", voice.participant_count)
        } else {
            tr("voice_invite_not_live")
        };
        hstack(12.0)
            .w_full()
            .items_center()
            .px(px(14.0))
            .py(px(10.0))
            .bg(theme.bg._200.opacity(0.6))
            .rounded(px(14.0))
            .child(
                div()
                    .size(px(36.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(theme.bg._200)
                    .children(IconUi::sized(
                        if voice.is_text { "number" } else { "speaker.wave.2.fill" },
                        px(17.0),
                        accent,
                        false,
                    )),
            )
            .child(
                vstack(2.0)
                    .items_start()
                    .min_w_0()
                    .child(
                        MainText::new(title)
                            .px(15.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.text.primary)
                            .line_limit(1),
                    )
                    .child(MainText::new(status).px(13.0).color(accent).line_limit(1)),
            )
            .child(spacer())
            .into_any_element()
    }

    fn join_button_title(preview: &ChatJoinPreview) -> String {
        if preview.voice.as_ref().is_some_and(|voice| !voice.is_text) && !preview.requires_approval {
            return tr("join_action_join_call");
        }
        if preview.requires_approval {
            return if preview.is_channel() { tr("join_action_request_channel") } else { tr("join_action_request") };
        }
        if preview.is_channel() { tr("join_action_join_channel") } else { tr("join_action_join") }
    }

    fn member_count_text(preview: &ChatJoinPreview) -> String {
        let n = preview.member_count;
        let (one, few, many) = if preview.is_channel() {
            (tr("chat_count_subscriber_one"), tr("chat_count_subscriber_few"), tr("chat_count_subscriber_many"))
        } else {
            (tr("chat_count_member_one"), tr("chat_count_member_few"), tr("chat_count_member_many"))
        };
        format!("{n} {}", plural_word(n, &one, &few, &many))
    }

    fn reason_text(join_status: &str) -> String {
        let status = trimming_whitespaces_and_newlines(join_status).to_lowercase();
        let is = |case: ChatInviteJoinStatus| status == case.wire();
        if is(ChatInviteJoinStatus::LinkExpired) {
            tr("join_link_expired")
        } else if is(ChatInviteJoinStatus::LinkInvalid) || is(ChatInviteJoinStatus::LinkRevoked) {
            tr("join_link_invalid")
        } else if is(ChatInviteJoinStatus::LinkUsageLimitReached) {
            tr("join_link_limit_reached")
        } else if is(ChatInviteJoinStatus::JoinRequestPending) {
            tr("join_request_pending")
        } else if is(ChatInviteJoinStatus::Banned) {
            tr("join_banned")
        } else if is(ChatInviteJoinStatus::VoiceUnavailable) {
            tr("join_voice_unavailable")
        } else if is(ChatInviteJoinStatus::VoiceForbidden) {
            tr("join_voice_forbidden")
        } else if is(ChatInviteJoinStatus::TopicUnavailable) {
            tr("join_topic_unavailable")
        } else if is(ChatInviteJoinStatus::TopicForbidden) {
            tr("join_topic_forbidden")
        } else if is(ChatInviteJoinStatus::MembersOnly) {
            tr("join_link_members_only")
        } else {
            tr("join_link_unavailable")
        }
    }
}

impl RenderOnce for JoinChatSheetContent {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let content = match &self.state.phase {
            JoinLinkSheetPhase::Loading => Self::loading_state(&theme),
            JoinLinkSheetPhase::Preview(preview) => self.preview_state(preview, &theme),
            JoinLinkSheetPhase::Empty => self.empty_state(&theme),
            JoinLinkSheetPhase::Failed(message, retryable) => self.error_state(message, *retryable, &theme),
        };
        vstack(0.0).w_full().px(px(22.0)).pt(px(10.0)).pb(px(8.0)).child(content)
    }
}

pub trait ChatJoinPreviewVoiceRefusal {
    fn is_voice_refusal(&self) -> bool;
}

impl ChatJoinPreviewVoiceRefusal for ChatJoinPreview {
    fn is_voice_refusal(&self) -> bool {
        let status = trimming_whitespaces_and_newlines(&self.join_status).to_lowercase();
        self.is_member
            && self.voice.is_some()
            && (status == ChatInviteJoinStatus::VoiceForbidden.wire()
                || status == ChatInviteJoinStatus::TopicForbidden.wire())
    }
}
