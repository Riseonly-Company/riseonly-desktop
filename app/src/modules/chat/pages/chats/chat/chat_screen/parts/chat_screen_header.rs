use std::path::PathBuf;
use std::rc::Rc;
use gpui::prelude::*;
use gpui::{AnyElement, App, Context, ElementId, FontWeight, SharedString, Window, div, px};
use rise_core::foundation::foundation_date::{Date, TimeInterval};
use rise_i18n::plural_word::plural_word;
use rise_i18n::tr;
use rise_platform::system_share::ShareItem;
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::page_header_toolbar_button::{PageHeaderIconButton, PageHeaderMetrics};
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::prelude::*;
use rise_ui::skeleton_ui::{SkeletonCircle, SkeletonLine, SkeletonShimmerExt};
use rise_ui::MainText;
use crate::app::router::navigation_types::{MainTab, RootRoute};
use crate::core::widgets::sheets::share_sheet::SystemShare;
use crate::modules::chat::components::chat::async_render::chat_performance_trace::ChatPerformanceTrace;
use crate::modules::chat::components::chat::composer::chat_composer_types::resign_chat_keyboard;
use crate::modules::chat::components::chat::chat_title_row::ChatTitleRow;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType};
use crate::modules::chat::stores::message::message_actions::message_types::GetMessageMessage;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;
use crate::modules::chat::stores::message::message_services::message_threads_services::ChannelPostRef;
use std::time::Duration;
use rise_canon::RiseGuideAnchor;
use rise_i18n::{FormatArg, tr_plural_with};
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::sai::core::websocket::sai_web_socket_manager::SaiWebSocketManager;
use crate::core::maintenance::maintenance_gate::MaintenanceGate;
use crate::modules::chat::components::chat::backdrop::chat_decorative_backdrop::{ChatBackdropLayers, ChatBackdropLayout};
use crate::modules::chat::components::chat::chat_logo::ChatLogo;
use crate::modules::chat::components::chat::chat_typing_indicator::ChatTypingIndicator;
use crate::modules::chat::pages::chats::chat::chat_roles::forum_member_grouping::ForumMemberGrouping;
use crate::modules::chat::pages::chats::chats::ChatsHeaderStatus;
use crate::modules::chat::stores::chats::chat_actions::chat_types::User;
use crate::modules::presence::stores::presence::presence_services::presence_services::PresenceServicesStore;
use super::super::chat_screen::*;
use super::super::chat_screen_support::*;

impl ChatScreen {
    pub(in super::super) fn thread_header_title(&self, cx: &mut App) -> String {
        let Some(root_id) = self.resolved_thread_root_id() else {
            return String::new();
        };
        let scope = self.message_scope(cx);
        let mut loaded = 0_i64;
        let mut root: Option<GetMessageMessage> = None;
        for message in message_services(cx, |store, cx| store.messages_in_memory(&scope, cx)) {
            if message.matches_message_id(root_id) {
                root = Some(message);
            } else if !message.is_system() {
                loaded += 1;
            }
        }
        let post = self.thread_channel_post.clone().or_else(|| ChannelPostRef::forward_of(root.as_ref()));
        let known = message_services(cx, |store, cx| {
            store
                .thread_total(root_id, cx)
                .or_else(|| post.as_ref().and_then(|post| store.comments_count(post, root_id, cx)))
                .unwrap_or(0)
        });
        let count = loaded.max(known);
        if count <= 0 {
            return tr("chat_thread_title_zero");
        }
        let one = tr("chat_thread_title_one");
        let few = tr("chat_thread_title_few");
        let many = tr("chat_thread_title_many");
        plural_word(count, &one, &few, &many).replace("{count}", &count.to_string())
    }

    pub(in super::super) fn is_top_chat_screen(&self, cx: &mut App) -> bool {
        let resolved = self.resolved_chat_id(cx);
        let Some(nav_store) = self.nav_store() else {
            return false;
        };
        let route_chat_id = match nav_store.read(cx).path.last() {
            Some(RootRoute::Chat { chat_id })
            | Some(RootRoute::ChatTopic { chat_id, .. })
            | Some(RootRoute::ForumAll { chat_id }) => chat_id.clone(),
            _ => return false,
        };
        route_chat_id == self.chat_id || route_chat_id.as_deref() == Some(resolved.as_str())
    }

    pub(in super::super) fn is_opened_beside_chat_list(&self, cx: &mut App) -> bool {
        let Some(nav_store) = self.nav_store() else {
            return false;
        };
        let nav = nav_store.read(cx);
        let beside = nav.chat_list_beside_stack() && nav.current_tab == MainTab::Chats && nav.path.len() == 1;
        beside && self.is_top_chat_screen(cx)
    }

    pub(in super::super) fn open_join_request_profile(&mut self, user_id: &str, tag: &str, cx: &mut Context<Self>) {
        self.show_join_requests_sheet = false;
        cx.notify();
        self.push_route(
            RootRoute::UserPage {
                user_id: (!user_id.is_empty()).then(|| user_id.to_owned()),
                tag: (!tag.is_empty()).then(|| tag.to_owned()),
            },
            cx,
        );
    }

    pub(in super::super) fn chat_header_center(&mut self, chat: &ChatInfo, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.is_thread_screen() {
            self.thread_header_center(window, cx)
        } else {
            self.chat_header_profile_button(chat, window, cx)
        }
    }

    pub(in super::super) fn chat_header_actions(&mut self, chat: &ChatInfo, cx: &mut Context<Self>) -> AnyElement {
        let this = cx.weak_entity();
        let mut actions = hstack(4.0_f32);
        if ChatPerformanceTrace::is_control_available() {
            let trace = ChatPerformanceTrace::shared();
            let system_image = if trace.is_finishing() {
                "hourglass.circle.fill"
            } else if trace.is_recording() {
                "stop.circle.fill"
            } else {
                "waveform.path.ecg"
            };
            let chat_id = self.resolved_chat_id(cx);
            actions = actions.child(
                PageHeaderIconButton::new(
                    "chat-performance-trace",
                    system_image,
                    Self::act(this.clone(), move |screen, window, cx| {
                        screen.toggle_chat_performance_trace(&chat_id, window, cx);
                    }),
                )
                .font_size(16.0)
                .disabled(trace.is_finishing()),
            );
        }
        if self.can_manage_bot_chat(cx)
            && let Some(bot_id) = self.bot_chat_details(cx).map(|bot| bot.id)
        {
            actions = actions.child(
                PageHeaderIconButton::new(
                    "chat-bot-settings",
                    "gearshape.fill",
                    Self::act(this.clone(), move |screen, window, cx| {
                        resign_chat_keyboard(window, cx);
                        screen.push_route(RootRoute::BotSettings { bot_id: bot_id.clone() }, cx);
                    }),
                )
                .font_size(16.0),
            );
        }
        actions = actions.child(self.chat_header_avatar_button(chat, cx));
        actions.into_any_element()
    }

    pub(in super::super) fn chat_header_avatar_button(&mut self, chat: &ChatInfo, cx: &mut Context<Self>) -> AnyElement {
        let size = PageHeaderMetrics::TOOLBAR_BUTTON_SIZE;
        if self.chat_metadata_loading(cx) {
            return div().flex_none().child(SkeletonCircle::new(size).skeleton_shimmer(None)).into_any_element();
        }
        let logo = ChatLogo::new(ElementId::Name(format!("chat-header-avatar-{}", chat.id).into()), chat.clone())
            .size(size)
            .icon_size(18.0)
            .show_online_indicator(false)
            .shows_worn_frame(true)
            .worn_frame_loops(true);
        let button = plain(ElementId::Name(format!("chat-header-avatar-button-{}", chat.id).into()))
            .flex_none()
            .rounded_full()
            .child(logo);
        if self.uses_context_preview_layout(cx) {
            return button.into_any_element();
        }
        let this = cx.weak_entity();
        let action_chat = chat.clone();
        button
            .on_press(Self::act(this, move |screen, window, cx| {
                resign_chat_keyboard(window, cx);
                let chat = &action_chat;
                if screen.is_message_request(cx) && chat.r#type == ChatType::Direct && !chat.participant.id.is_empty() {
                    let tag = (!chat.participant.tag.is_empty()).then(|| chat.participant.tag.clone());
                    screen.push_route(RootRoute::UserPage { user_id: Some(chat.participant.id.clone()), tag }, cx);
                } else {
                    screen.push_route(RootRoute::ChatProfile { chat_id: Some(chat.id.clone()) }, cx);
                }
            }))
            .into_any_element()
    }

    pub(in super::super) fn toggle_chat_performance_trace(&mut self, chat_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let trace = ChatPerformanceTrace::shared();
        if trace.is_finishing() || chat_id.is_empty() {
            return;
        }
        if trace.is_recording() {
            let this = cx.weak_entity();
            let async_cx = cx.to_async();
            trace.stop(
                "header_button",
                Some(Box::new(move |url: Option<PathBuf>| {
                    let _ = async_cx.update(|cx| {
                        let _ = this.update(cx, |screen, cx| {
                            screen.chat_performance_trace_revision = screen.chat_performance_trace_revision.wrapping_add(1);
                            cx.notify();
                        });
                        let Some(url) = url else {
                            return;
                        };
                        let _ = SystemShare::present(vec![ShareItem::File(url)], None, cx);
                    });
                })),
            );
            self.chat_performance_trace_revision = self.chat_performance_trace_revision.wrapping_add(1);
            cx.notify();
        } else if trace.start(chat_id, false, window, cx) {
            self.chat_performance_trace_revision = self.chat_performance_trace_revision.wrapping_add(1);
            cx.notify();
        }
    }

    pub(in super::super) fn thread_header_center(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        let title = self.thread_header_title(cx);
        div()
            .py(px(6.0))
            .child(MainText::new(title).px(16.0).font_weight(FontWeight::SEMIBOLD).color(theme.text.primary).number_of_lines(1))
            .liquid_glass_nav_pill(14.0, None)
    }

    pub(in super::super) fn chat_header_profile_button(&mut self, chat: &ChatInfo, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let this = cx.weak_entity();
        let action_chat = chat.clone();
        let header_topic_title = self.header_topic_title(cx);
        let title: AnyElement = match header_topic_title {
            Some(topic_title) => div()
                .min_w_0()
                .max_w_full()
                .child(MainText::new(topic_title).px(16.0).font_weight(FontWeight::SEMIBOLD).number_of_lines(1))
                .into_any_element(),
            None => div()
                .min_w_0()
                .max_w_full()
                .child(ChatTitleRow::new(chat.clone()).title_px(16.0).fav_px(16.0).icon_size(11.0).font_weight(FontWeight::SEMIBOLD))
                .into_any_element(),
        };
        let scoped = window.use_keyed_state(
            ElementId::Name(format!("chat-header-subtitle-{}", chat.id).into()),
            cx,
            |_, cx| ChatHeaderScopedContent::new(cx),
        );
        let model_chat = chat.clone();
        let model_screen = this.clone();
        let model: Rc<dyn Fn(&mut App) -> Option<ChatHeaderSubtitleModel>> = Rc::new(move |cx: &mut App| {
            model_screen.update(cx, |screen, cx| screen.header_subtitle_model(&model_chat, cx)).ok()
        });
        let chat_id = chat.id.clone();
        let scope_screen = this.clone();
        scoped.update(cx, |scoped, cx| scoped.configure(&chat_id, scope_screen, model, cx));
        let metadata_loading = self.chat_metadata_loading(cx);
        LiquidGlassButton::new("chat-header-profile", LiquidGlassButtonShape::Capsule)
            .horizontal_padding(10.0)
            .vertical_padding(6.0)
            .on_press(Self::act(this, move |screen, window, cx| {
                resign_chat_keyboard(window, cx);
                let chat = &action_chat;
                if screen.is_message_request(cx) && chat.r#type == ChatType::Direct && !chat.participant.id.is_empty() {
                    let tag = (!chat.participant.tag.is_empty()).then(|| chat.participant.tag.clone());
                    screen.push_route(RootRoute::UserPage { user_id: Some(chat.participant.id.clone()), tag }, cx);
                    return;
                }
                let route = if let Some(topic_id) = screen.header_topic_id(cx) {
                    RootRoute::TopicProfile { chat_id: Some(chat.id.clone()), topic_id: Some(topic_id) }
                } else if screen.is_topics_mode(cx) && screen.state.selected_topic_tab == ChatTopicTab::General {
                    RootRoute::GeneralProfile { chat_id: Some(chat.id.clone()) }
                } else {
                    RootRoute::ChatProfile { chat_id: Some(chat.id.clone()) }
                };
                screen.push_route(route, cx);
            }))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .min_w_0()
                    .px(px(4.0))
                    .when(metadata_loading, |column| {
                        column.h(px(38.0)).child(
                            div()
                                .flex()
                                .flex_col()
                                .items_center()
                                .justify_center()
                                .gap(px(6.0))
                                .child(SkeletonLine::new(110.0, 12.0))
                                .child(SkeletonLine::new(72.0, 9.0))
                                .skeleton_shimmer(None),
                        )
                    })
                    .when(!metadata_loading, |column| {
                        column
                            .min_h(px(38.0))
                            .gap(px(2.0))
                            .child(title)
                            .child(div().min_w_0().max_w_full().child(scoped))
                    }),
            )
            .guide_anchor(RiseGuideAnchor::ChatHeaderTitle)
            .into_any_element()
    }

    pub(in super::super) fn header_subtitle_model(&mut self, chat: &ChatInfo, cx: &mut Context<Self>) -> ChatHeaderSubtitleModel {
        let is_favourites = chat.r#type == ChatType::Favourites;
        let messages = MessageServicesStore::shared(cx);
        let typing_key = self.typing_key(chat, cx);
        let typists = if is_favourites { Vec::new() } else { messages.read(cx).typists(&typing_key) };
        let bot_activity_status = if is_favourites { None } else { messages.read(cx).bot_activity_status(&typing_key) };
        let presence = PresenceServicesStore::shared(cx);
        let in_chat = chat.r#type == ChatType::Direct && presence.read(cx).is_in_chat(&chat.participant.id, &chat.id);
        let refresh_timestamp = self.header_refresh_timestamp(chat, cx);
        let is_closed_for_maintenance = MaintenanceGate::shared(cx).read(cx).snapshot().is_closed;
        let socket = SaiWebSocketManager::shared(cx);
        let socket_manager = socket.read(cx);
        let connection_status = ChatsHeaderStatus::resolve(
            socket_manager.phase(),
            false,
            is_closed_for_maintenance,
            socket_manager.is_reconnecting_after_failure(),
        );
        let status = |text: String, accent: bool| ChatHeaderSubtitleText::Status { text, accent };
        let text = if !matches!(connection_status, ChatsHeaderStatus::Title) {
            status(connection_status.text(), false)
        } else if self.chat_metadata_loading(cx) {
            status(tr("common_loading"), false)
        } else if !typists.is_empty() {
            if let Some(activity) = bot_activity_status {
                ChatHeaderSubtitleText::Typing { text: activity, trailing: None }
            } else {
                let (names, verb) = self.typing_label(chat, &typists, cx);
                match names {
                    Some(names) => ChatHeaderSubtitleText::Typing { text: names, trailing: Some(verb) },
                    None => ChatHeaderSubtitleText::Typing { text: verb, trailing: None },
                }
            }
        } else if in_chat {
            status(tr("chat_status_in_chat"), true)
        } else {
            let is_online = chat.r#type == ChatType::Direct
                && presence.read(cx).is_online(&chat.participant.id, chat.participant.is_online);
            status(self.header_status(chat, cx), is_online)
        };
        ChatHeaderSubtitleModel { text, refresh_timestamp }
    }

    pub(in super::super) fn header_subtitle_element(model: &ChatHeaderSubtitleModel, chat_id: &str, cx: &mut App) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        match &model.text {
            ChatHeaderSubtitleText::Status { text, .. } if text.is_empty() => gpui::Empty.into_any_element(),
            ChatHeaderSubtitleText::Status { text, accent } => MainText::new(text.clone())
                .px(12.0)
                .color(if *accent { theme.semantic.success_100 } else { theme.text.secondary })
                .number_of_lines(1)
                .into_any_element(),
            ChatHeaderSubtitleText::Typing { text, trailing } => {
                let indicator_id = ElementId::Name(format!("chat-header-typing-{chat_id}").into());
                let indicator = ChatTypingIndicator::new(indicator_id, text.clone(), theme.semantic.success_100);
                match trailing {
                    Some(trailing) => indicator.trailing(SharedString::from(trailing.clone())).into_any_element(),
                    None => indicator.into_any_element(),
                }
            }
        }
    }

    /// typing is kept per topic and thread, the aggregate feed uses the chat key
    pub(in super::super) fn typing_key(&self, chat: &ChatInfo, cx: &mut App) -> String {
        let topic_id = self.resolved_topic_id(cx);
        MessageServicesStore::message_scope(&chat.id, topic_id.as_deref(), self.resolved_thread_root_id())
    }

    pub(in super::super) fn header_refresh_timestamp(&self, chat: &ChatInfo, cx: &mut App) -> Option<TimeInterval> {
        let _ = self.header_status_refresh_revision;
        if chat.r#type == ChatType::Favourites {
            return None;
        }
        let typing_key = self.typing_key(chat, cx);
        let typing_timestamps: Vec<TimeInterval> = MessageServicesStore::shared(cx)
            .read(cx)
            .typing_peers_by_chat
            .get(&typing_key)
            .map(|peers| peers.iter().filter(|peer| !peer.name.is_empty()).map(|peer| peer.at).collect())
            .unwrap_or_default();
        let presence = PresenceServicesStore::shared(cx);
        let is_online = chat.r#type == ChatType::Direct
            && presence.read(cx).is_online(&chat.participant.id, chat.participant.is_online);
        let last_seen = if chat.r#type == ChatType::Direct {
            presence.read(cx).last_seen(&chat.participant.id, chat.participant.last_seen)
        } else {
            None
        };
        ChatHeaderRefreshSchedule::next_refresh_timestamp(
            &typing_timestamps,
            MessageServicesStore::TYPING_TTL_SECONDS,
            is_online,
            last_seen,
            Date::now().time_interval_since_1970(),
            Some(MaintenanceGate::shared(cx).read(cx).server_now().time_interval_since_1970()),
        )
    }

    pub(in super::super) fn wait_for_header_refresh(&mut self, timestamp: Option<TimeInterval>, cx: &mut Context<Self>) {
        self.header_refresh_task = timestamp.map(|timestamp| {
            let delay = (timestamp - Date::now().time_interval_since_1970()).max(0.0);
            cx.spawn(async move |this, cx| {
                if delay > 0.0 {
                    cx.background_executor().timer(Duration::from_secs_f64(delay)).await;
                }
                let _ = this.update(cx, |screen, cx| {
                    screen.header_status_refresh_revision = screen.header_status_refresh_revision.wrapping_add(1);
                    cx.notify();
                });
            })
        });
    }

    /// names that may shorten in a narrow header and the verb that stays whole
    pub(in super::super) fn typing_label(&self, chat: &ChatInfo, typists: &[String], cx: &mut App) -> (Option<String>, String) {
        let typists: Vec<&str> = typists.iter().map(String::as_str).filter(|name| !name.is_empty()).collect();
        let typing_key = self.typing_key(chat, cx);
        let action = MessageServicesStore::shared(cx).read(cx).typing_action(&typing_key);
        let single = MessageServicesStore::typing_verb(action.as_deref(), false);
        match chat.r#type {
            ChatType::Group | ChatType::Channel => {
                if typists.is_empty() {
                    (None, single)
                } else if typists.len() == 1 {
                    (Some(typists[0].to_owned()), single)
                } else {
                    (Some(typists[..2].join(", ")), MessageServicesStore::typing_verb(action.as_deref(), true))
                }
            }
            ChatType::Direct | ChatType::Favourites => (None, single),
        }
    }

    pub(in super::super) fn header_status(&self, chat: &ChatInfo, cx: &mut App) -> String {
        if chat.r#type == ChatType::Favourites {
            return tr("chat_status_favourites");
        }
        let typing_key = self.typing_key(chat, cx);
        let typists = MessageServicesStore::shared(cx).read(cx).typists(&typing_key);
        if !typists.is_empty() {
            let (names, verb) = self.typing_label(chat, &typists, cx);
            return names.map(|names| format!("{names} {verb}")).unwrap_or(verb);
        }
        match chat.r#type {
            ChatType::Direct => {
                if chat.participant.is_deleted {
                    return tr("chat_status_long_ago");
                }
                if chat.has_no_direct_peer() {
                    return String::new();
                }
                PresenceServicesStore::shared(cx).update(cx, |store, cx| store.status_text_for_chat(chat, cx))
            }
            ChatType::Group => ForumMemberGrouping::members_line(&chat.id, chat.member_count, None, cx),
            ChatType::Channel => {
                let count = RiseFormat::compact_count(chat.member_count);
                tr_plural_with("chat_subscribers_count", chat.member_count, FormatArg::Text(count.as_str().into()))
            }
            ChatType::Favourites => tr("chat_status_favourites"),
        }
    }

    pub(in super::super) fn open_sender_chat_profile(&mut self, participant: &User, window: &mut Window, cx: &mut Context<Self>) {
        if participant.is_deleted {
            return;
        }
        resign_chat_keyboard(window, cx);
        let chat_id = self.resolved_chat_id(cx);
        if participant.id == chat_id {
            self.push_route(RootRoute::ChatProfile { chat_id: Some(chat_id) }, cx);
            return;
        }
        self.push_route(
            RootRoute::UserPage { user_id: Some(participant.id.clone()), tag: Some(participant.tag.clone()) },
            cx,
        );
    }

    /// a forum has no shared wallpaper
    /// all draws the chat one, general and every topic draw only their own with no fallback
    pub(in super::super) fn chat_backdrop(&mut self, chat: &ChatInfo, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let cx: &mut App = cx;
        let topic_wallpaper = |topic_id: &str, cx: &mut App| {
            chat_services(cx, |store, cx| store.topic_from_any_cache(&chat.id, topic_id, cx)).and_then(|topic| topic.wallpaper)
        };
        let wallpaper = if self.is_topics_mode(cx) {
            match &self.state.selected_topic_tab {
                ChatTopicTab::All => chat.wallpaper.clone(),
                ChatTopicTab::General => {
                    chat_services(cx, |store, cx| store.general_topic(&chat.id, cx)).and_then(|topic| topic.wallpaper)
                }
                ChatTopicTab::Topic(topic_id) => topic_wallpaper(topic_id, cx),
            }
        } else if let ChatTopicTab::Topic(topic_id) = &self.state.selected_topic_tab {
            topic_wallpaper(topic_id, cx).or_else(|| chat.wallpaper.clone())
        } else {
            chat.wallpaper.clone()
        };
        ChatBackdropLayers::new(ChatBackdropLayout::FullBleed)
            .chat_id(Some(chat.id.clone()))
            .chat_wallpaper(wallpaper)
            .into_any_element()
    }
}
