use std::collections::HashSet;
use std::rc::Rc;
use gpui::prelude::*;
use gpui::{AnyElement, App, Context, FontWeight, Pixels, Point, SharedString, Window, div, px};
use rise_core::foundation::foundation_calendar::Calendar;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::{Locale, tr};
use rise_platform::haptics::selection_changed;
use rise_theme::AppTheme;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape, LiquidGlassMenuLabel};
use rise_ui::swiftui::motion::ease_out;
use rise_ui::swiftui::numeric_text::NumericText;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::IconUi;
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::sai::core::utilities::get_sai_server_id::GetSaiServerId;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuContainer;
use crate::modules::chat::components::chat::async_render::rise_message_list_view::RiseMessageScrollRequest;
use crate::modules::chat::engine::messages::rise_message_scope::RiseMessageJumpOutcome;
use crate::modules::chat::engine::messages::rise_message_unread_marks::RiseUnreadMarkKind;
use crate::modules::chat::shared::chat_message_jump_progress::ChatMessageJumpProgress;
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageJumpCompletion;
use crate::modules::chat::stores::message::message_services::message_unread_marks_services::MessageUnreadMarksPresentation;
use crate::modules::chat::components::chat::composer::chat_composer_types::{ChatBottomBarMetrics, resign_chat_keyboard};
use crate::modules::chat::shared::chat_message_jump_presenter::ChatMessageJumpPresenter;
use crate::modules::chat::shared::chat_permission_resolution::ChatPermissionResolution;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatType;
use crate::modules::chat::stores::message::message_actions::message_types::{GetMessageMessage, MessageContentType, MessageDeliveryStatus};
use crate::modules::chat::components::chat::async_render::rise_message_list_view::RiseMessageScrollDestination;
use crate::modules::chat::engine::messages::rise_chat_calendar::{RiseChatCalendarDay, RiseChatCalendarWindow};
use crate::modules::chat::shared::chat_message_jump_progress::ChatMessageJumpProgressScope;
use crate::modules::chat::stores::message::message_actions::message_actions::MessageActionFlagHandler;
use crate::modules::chat::stores::message::message_actions::message_types::MessageHistoryDirection;
use crate::modules::chat::stores::message::message_services::message_pagination_trace::MessagePaginationTrace;
use super::super::chat_screen::*;
use super::super::chat_screen_support::*;

impl ChatScreen {
    /// route identity is independent from message_scope
    /// topic selection and a temp chat id can change mid build
    /// matching the origin route stops a late first frame callback from acting on another screen
    pub(in super::super) fn first_frame_navigation_route(&self) -> RootRoute {
        if self.thread_root_id.is_some() {
            return RootRoute::ChatThread { chat_id: self.chat_id.clone(), root_id: self.thread_root_id.clone() };
        }
        if self.initial_all_view {
            return RootRoute::ForumAll { chat_id: self.chat_id.clone() };
        }
        if self.topic_id.is_some() {
            return RootRoute::ChatTopic { chat_id: self.chat_id.clone(), topic_id: self.topic_id.clone() };
        }
        RootRoute::Chat { chat_id: self.chat_id.clone() }
    }

    pub(in super::super) fn report_partial_read_progress(&self, visible_message_ids: Vec<String>, cx: &mut App) {
        if self.is_context_preview(cx)
            || self.is_message_request(cx)
            || self.is_community_outsider(cx)
            || self.message_viewport_chrome.read(cx).is_at_latest()
            || self.active_unread_count(cx) <= 0
            || visible_message_ids.is_empty()
        {
            return;
        }
        let scope = self.message_scope(cx);
        let Some(snapshot) = message_services(cx, |store, cx| store.message_scope_presentation_snapshot(&scope, cx)) else {
            return;
        };
        let account_id = account_id(cx);
        let visible: HashSet<String> = visible_message_ids.into_iter().collect();
        let on_screen: Vec<_> = snapshot.slice.messages.iter().filter(|message| visible.contains(&message.sai_server_id())).collect();
        if on_screen.len() <= 1 {
            return;
        }
        let fully_shown = &on_screen[..on_screen.len() - 1];
        let Some(reached) = fully_shown.iter().rev().find(|message| !message.sender_id.is_empty() && message.sender_id != account_id) else {
            return;
        };
        let chat_id = self.resolved_chat_id(cx);
        let topic_id = self.read_topic_id(cx);
        let (message_id, created_at) = (reached.sai_server_id(), reached.created_at);
        message_interactions(cx, |store, cx| {
            store.mark_chat_read_up_to(&chat_id, topic_id.as_deref(), &message_id, created_at, cx);
        });
    }

    pub(in super::super) fn active_unread_count(&self, cx: &mut App) -> i64 {
        let chat_id = self.resolved_chat_id(cx);
        let topic_id = self.read_topic_id(cx);
        chat_services(cx, |store, cx| store.unread_count_topic_id(&chat_id, topic_id.as_deref(), cx))
    }

    /// read state only moves forward so the newest incoming row is the one to ask
    /// is_read true there means anyone read it, false proves this account hasnt
    pub(in super::super) fn newest_incoming_is_unread(messages: &[GetMessageMessage], account_id: &str) -> bool {
        messages
            .iter()
            .rev()
            .find(|message| !message.sender_id.is_empty() && message.sender_id != account_id)
            .is_some_and(|incoming| !incoming.is_read)
    }

    pub(in super::super) fn unread_divider_candidate(
        messages: &[GetMessageMessage],
        count: i64,
        account_id: &str,
        own_identity: Option<&str>,
    ) -> Option<String> {
        if count <= 0 || account_id.is_empty() {
            return None;
        }
        let mut seen = 0;
        for message in messages.iter().rev() {
            if message.sender_id.is_empty()
                || message.sender_id == account_id
                || own_identity == Some(message.sender_id.as_str())
                || message.content_type == MessageContentType::System
                || message.is_temp
            {
                continue;
            }
            seen += 1;
            if seen == count {
                return Some(message.id.clone());
            }
        }
        None
    }

    pub(in super::super) fn unread_divider_message_id(&self, messages: &[GetMessageMessage], cx: &mut App) -> Option<String> {
        if self.unread_divider.scope.as_deref() != Some(self.message_scope(cx).as_str()) {
            return None;
        }
        if let Some(message_id) = &self.unread_divider.message_id {
            return Some(message_id.clone());
        }
        if self.unread_divider.count <= 0 {
            return None;
        }
        let account_id = account_id(cx);
        if !self.window_reaches_chat_last_incoming(messages, &account_id, cx) {
            return None;
        }
        let own_identity = self.own_channel_identity(cx);
        Self::unread_divider_candidate(messages, self.unread_divider.count, &account_id, own_identity.as_deref())
    }

    pub(in super::super) fn own_channel_identity(&self, cx: &mut App) -> Option<String> {
        let chat = self.resolved_chat(cx);
        (chat.r#type == ChatType::Channel
            && !chat.id.is_empty()
            && ChatPermissionResolution::can_post_to_channel(chat.role.as_deref()))
        .then_some(chat.id)
    }

    pub(in super::super) fn window_reaches_chat_last_incoming(&self, messages: &[GetMessageMessage], account_id: &str, cx: &mut App) -> bool {
        if self.is_topics_mode(cx) || self.is_thread_screen() {
            return true;
        }
        let chat = self.resolved_chat(cx);
        let Some(last) = chat.last_message.as_ref() else {
            return true;
        };
        if last.sender_id.is_empty() || last.sender_id == account_id || last.content_type == MessageContentType::System {
            return true;
        }
        let ids: Vec<&str> = [last.id.as_str(), last.server_id.as_deref().unwrap_or("")]
            .into_iter()
            .filter(|id| !id.is_empty())
            .collect();
        if ids.is_empty() {
            return true;
        }
        messages.iter().rev().any(|message| ids.iter().any(|id| message.matches_message_id(id)))
    }

    pub(in super::super) fn latest_own_taken_send(messages: &[GetMessageMessage], account_id: &str) -> Option<OwnSend> {
        if account_id.is_empty() {
            return None;
        }
        messages
            .iter()
            .rev()
            .find(|message| {
                message.sender_id == account_id
                    && message.delivery_status != MessageDeliveryStatus::Failed
                    && message.delivery_status != MessageDeliveryStatus::Sending
            })
            .map(|own| OwnSend { id: own.id.clone(), sent_at: own.created_at })
    }

    pub(in super::super) fn read_trigger(latest: Option<&GetMessageMessage>, messages: &[GetMessageMessage], account_id: &str) -> ReadTrigger {
        ReadTrigger {
            latest_id: latest.map(|message| message.id.clone()),
            latest_is_own: latest.is_some_and(|message| message.sender_id == account_id),
            newest_incoming_id: messages
                .iter()
                .rev()
                .find(|message| !message.sender_id.is_empty() && message.sender_id != account_id)
                .map(|message| message.id.clone()),
        }
    }

    /// the badge behind active_unread_count can lag while this chat covers the list
    /// so arriving at the bottom also asks the rows on screen
    pub(in super::super) fn open_scope_has_unread_incoming(&self, cx: &mut App) -> bool {
        if self.active_unread_count(cx) > 0 {
            return true;
        }
        let scope = self.message_scope(cx);
        let Some(snapshot) = message_services(cx, |store, cx| store.message_scope_presentation_snapshot(&scope, cx)) else {
            return false;
        };
        Self::newest_incoming_is_unread(&snapshot.slice.messages, &account_id(cx))
    }

    /// same identity the list uses to start a new presentation, the scope and the open epoch
    pub(in super::super) fn viewport_presentation(&self, cx: &mut App) -> String {
        let epoch = self.navigation_open_epoch.map(|epoch| epoch.to_string()).unwrap_or_default();
        format!("{}|{epoch}", self.message_scope(cx))
    }

    pub(in super::super) fn jump_to_latest_button(&mut self, theme: &AppTheme, cx: &mut Context<Self>) -> AnyElement {
        let unread_count = self.active_unread_count(cx).max(0);
        let this = cx.weak_entity();
        let label = tr("chat_scroll_to_bottom").to_string();
        let button = LiquidGlassButton::new("chat-jump-to-latest", LiquidGlassButtonShape::Circle)
            .size(ChatBottomBarMetrics::CIRCLE)
            .accessibility_label(Some(label.clone()))
            .on_press(Self::act(this, |screen, window, cx| screen.jump_to_latest_messages(window, cx)))
            .children(IconUi::sized("chevron.down", px(19.0), theme.text.primary, false));
        div()
            .id("chat-jump-to-latest-host")
            .relative()
            .tooltip_label(label)
            .child(button)
            .children((unread_count > 0).then(|| Self::chrome_count_pill("chat-chrome-count-unread", unread_count, theme)))
            .into_any_element()
    }

    pub(in super::super) fn unread_marks_task_id(&self, cx: &mut App) -> String {
        let topic_id = self.read_topic_id(cx).unwrap_or_else(|| "<chat>".to_owned());
        format!("{}|{}|{}", self.resolved_chat_id(cx), topic_id, self.is_pending_chat(cx))
    }

    pub(in super::super) fn unread_marks_presentation(&self, cx: &mut App) -> MessageUnreadMarksPresentation {
        let chat_id = self.resolved_chat_id(cx);
        let topic_id = self.read_topic_id(cx);
        message_services(cx, |store, cx| store.unread_marks(&chat_id, topic_id.as_deref(), cx))
    }

    fn shown_unread_marks(&self, chrome_visible: bool, cx: &mut App) -> MessageUnreadMarksPresentation {
        if chrome_visible && !self.is_message_request(cx) && !self.is_community_outsider(cx) {
            self.unread_marks_presentation(cx)
        } else {
            MessageUnreadMarksPresentation::default()
        }
    }

    pub(in super::super) fn unread_mark_buttons_lift(&self, cx: &mut App) -> f32 {
        if self.state.is_message_search_active || self.uses_context_preview_layout(cx) {
            return 0.0;
        }
        let chrome_visible = ChatJumpToLatestPresentationPolicy::is_visible(
            true,
            self.state.is_message_selection_active,
            self.show_bot_chat_menu,
        );
        let marks = self.shown_unread_marks(chrome_visible, cx);
        let shown = [marks.reactions_count, marks.replies_count, marks.mentions_count].iter().filter(|count| **count > 0).count();
        shown as f32 * (ChatBottomBarMetrics::CIRCLE + ChatBottomBarMetrics::ROW_GAP)
    }

    pub(in super::super) fn message_navigation_buttons(&mut self, theme: &AppTheme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.state.is_message_search_active {
            self.message_search_step_buttons(theme, window, cx)
        } else {
            let shows_jump_to_bottom =
                ChatViewportChromeReader::new(self.message_viewport_chrome.clone()).content(cx, |shows| shows);
            self.standard_message_navigation_buttons(theme, shows_jump_to_bottom, window, cx)
        }
    }

    pub(in super::super) fn standard_message_navigation_buttons(
        &mut self,
        theme: &AppTheme,
        shows_jump_to_bottom: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let chrome_visible = ChatJumpToLatestPresentationPolicy::is_visible(
            true,
            self.state.is_message_selection_active,
            self.show_bot_chat_menu,
        );
        let shows_down = chrome_visible && shows_jump_to_bottom;
        let marks = self.shown_unread_marks(chrome_visible, cx);
        let reactions_count = marks.reactions_count;
        let replies_count = marks.replies_count;
        let mentions_count = marks.mentions_count;
        let spec = ChatMessageNavigationButtonMotion::animation();
        let transition = ChatMessageNavigationButtonMotion::transition();
        let reactions_theme = theme.clone();
        let reactions = self.transitioned(
            "chat-nav-reactions",
            reactions_count > 0,
            transition.clone(),
            spec,
            window,
            cx,
            move |screen, _, cx| {
                div().child(screen.unread_mark_button(RiseUnreadMarkKind::Reaction, reactions_count, &reactions_theme, cx))
            },
        );
        let replies_theme = theme.clone();
        let replies = self.transitioned(
            "chat-nav-replies",
            replies_count > 0,
            transition.clone(),
            spec,
            window,
            cx,
            move |screen, _, cx| {
                div().child(screen.unread_mark_button(RiseUnreadMarkKind::Reply, replies_count, &replies_theme, cx))
            },
        );
        let mentions_theme = theme.clone();
        let mentions = self.transitioned(
            "chat-nav-mentions",
            mentions_count > 0,
            transition.clone(),
            spec,
            window,
            cx,
            move |screen, _, cx| {
                div().child(screen.unread_mark_button(RiseUnreadMarkKind::Mention, mentions_count, &mentions_theme, cx))
            },
        );
        let down_theme = theme.clone();
        let down = self.transitioned("chat-nav-down", shows_down, transition, spec, window, cx, move |screen, _, cx| {
            div().child(screen.jump_to_latest_button(&down_theme, cx))
        });
        vstack(ChatBottomBarMetrics::ROW_GAP)
            .items_end()
            .children(reactions)
            .children(replies)
            .children(mentions)
            .children(down)
            .into_any_element()
    }

    pub(in super::super) fn unread_mark_button(&mut self, kind: RiseUnreadMarkKind, count: i64, theme: &AppTheme, cx: &mut Context<Self>) -> AnyElement {
        let face = UnreadMarkButtonFace::new(kind);
        let this = cx.weak_entity();
        let action_title = face.read_all_title;
        let label = face.accessibility_label;
        let system_image = face.system_image;
        let icon_size = face.icon_size;
        let content_theme = theme.clone();
        HoldContextMenuContainer::new(
            match kind {
                RiseUnreadMarkKind::Mention => "chat-unread-mentions",
                RiseUnreadMarkKind::Reply => "chat-unread-replies",
                RiseUnreadMarkKind::Reaction => "chat-unread-reactions",
            },
            HoldContextMenuAlignment::Trailing,
            Rc::new(move |_: &App| {
                let mut action = HoldContextMenuAction::new("read_all", action_title.clone());
                action.system_image = Some("checkmark.circle".to_owned());
                vec![action]
            }),
            Rc::new(move |_: &mut Window, _: &mut App| {
                div()
                    .id(SharedString::from(label.clone()))
                    .relative()
                    .tooltip_label(label.clone())
                    .child(
                        LiquidGlassMenuLabel::new()
                            .shape(LiquidGlassButtonShape::Circle)
                            .size(ChatBottomBarMetrics::CIRCLE)
                            .children(IconUi::sized(
                                system_image,
                                px(icon_size),
                                content_theme.text.primary,
                                system_image.ends_with(".fill"),
                            )),
                    )
                    .child(Self::chrome_count_pill(
                        match kind {
                            RiseUnreadMarkKind::Mention => "chat-chrome-count-mentions",
                            RiseUnreadMarkKind::Reply => "chat-chrome-count-replies",
                            RiseUnreadMarkKind::Reaction => "chat-chrome-count-reactions",
                        },
                        count,
                        &content_theme,
                    ))
                    .into_any_element()
            }),
        )
        .show_emoji_list(false)
        .on_action(Rc::new({
            let this = this.clone();
            move |_: String, _: &mut Window, cx: &mut App| {
                let _ = this.update(cx, |screen, cx| screen.read_all_unread_marks(kind, cx));
            }
        }))
        .on_tap(Rc::new(move |_: Point<Pixels>, window: &mut Window, cx: &mut App| {
            let _ = this.update(cx, |screen, cx| screen.jump_to_next_unread_mark(kind, window, cx));
        }))
        .into_any_element()
    }

    pub(in super::super) fn chrome_count_pill(id: &'static str, count: i64, theme: &AppTheme) -> AnyElement {
        let text = if count > 999 { "999+".to_owned() } else { count.to_string() };
        div()
            .absolute()
            .top(px(-4.0))
            .right(px(-4.0))
            .flex()
            .items_center()
            .justify_center()
            .min_w(px(18.0))
            .min_h(px(18.0))
            .px(px(5.0))
            .rounded_full()
            .bg(theme.primary._100)
            .border(px(1.5))
            .border_color(theme.bg._100)
            .text_size(px(10.0))
            .font_weight(FontWeight::BOLD)
            .text_color(theme.content_on(theme.primary._100))
            .child(NumericText::new(id, text).counts_down(true).animation(ease_out(0.2), count))
            .into_any_element()
    }

    pub(in super::super) fn jump_to_next_unread_mark(&mut self, kind: RiseUnreadMarkKind, window: &mut Window, cx: &mut Context<Self>) {
        let chat_id = self.resolved_chat_id(cx);
        if chat_id.is_empty() {
            return;
        }
        let topic_id = self.read_topic_id(cx);
        let this = cx.weak_entity();
        let handle = window.window_handle();
        let completion: Rc<dyn Fn(Option<String>, &mut App)> = Rc::new(move |target: Option<String>, cx: &mut App| {
            let Some(target) = target.filter(|target| !target.is_empty()) else {
                return;
            };
            Self::deliver_in_window(&this, handle, cx, move |screen, window, cx| {
                screen.jump_to_message_id(&target, window, cx);
            });
        });
        message_interactions(cx, |store, cx| {
            store.next_unread_mark(&chat_id, topic_id.as_deref(), kind, completion, cx);
        });
    }

    pub(in super::super) fn read_all_unread_marks(&mut self, kind: RiseUnreadMarkKind, cx: &mut Context<Self>) {
        let chat_id = self.resolved_chat_id(cx);
        if chat_id.is_empty() {
            return;
        }
        let topic_id = self.read_topic_id(cx);
        message_interactions(cx, |store, cx| store.read_all_unread_marks(&chat_id, topic_id.as_deref(), kind, cx));
        let _ = selection_changed();
    }

    pub(in super::super) fn handle_message_viewport_state(
        &mut self,
        is_at_latest: bool,
        shows_jump_to_bottom: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let presentation = self.viewport_presentation(cx);
        self.message_viewport_chrome.update(cx, |chrome, cx| {
            chrome.update_presentation(is_at_latest, shows_jump_to_bottom, presentation, cx);
        });
        if !self.is_context_preview(cx)
            && is_at_latest
            && !self.is_message_request(cx)
            && !self.is_community_outsider(cx)
            && self.open_scope_has_unread_incoming(cx)
        {
            self.mark_open_chat_as_read(cx);
        }
    }

    pub(in super::super) fn jump_to_pinned_message(&mut self, message: &GetMessageMessage, window: &mut Window, cx: &mut Context<Self>) {
        let message_id = trimming_whitespaces_and_newlines(&message.sai_server_id()).to_owned();
        let chat_id = self.resolved_chat_id(cx);
        if chat_id.is_empty() || message_id.is_empty() || GetSaiServerId::is_temp_id(Some(message_id.as_str())) {
            return;
        }
        if let Some(tab) = self.topic_tab_admitting(message.topic_id.as_deref(), cx) {
            self.redeliver_message_jump(message_id, tab, window, cx);
            return;
        }
        self.jump_to_message_id(&message_id, window, cx);
    }

    /// hands a travel to the message load task by settling the forum tab
    /// flipping the tab flips the task id and its run carries the target as the opening request
    pub(in super::super) fn redeliver_message_jump(&mut self, message_id: String, tab: ChatTopicTab, window: &mut Window, cx: &mut Context<Self>) {
        ChatMessageJumpProgress::shared(cx).update(cx, |progress, cx| progress.abandon(cx));
        self.pending_pinned_jump_id = None;
        self.delivered_jump_message_id = Some(message_id);
        self.select_topic_tab(tab, window, cx);
    }

    /// performs a jump left behind by a screen that couldnt perform it
    /// used when this screen is already open, eg back from the chat profile
    /// the load task doesnt re run
    pub(in super::super) fn consume_pending_message_jump(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let task_id_before_settle = self.message_load_task_id(cx);
        let Some(target) = self.consume_pending_message_jump_target(window, cx) else {
            return;
        };
        if self.message_load_task_id(cx) != task_id_before_settle {
            return;
        }
        self.jump_to_message_id(&target, window, cx);
    }

    /// takes the pending target and settles its tab without starting the travel
    /// the load task needs the target before it opens the scope so opening and travelling are one request
    pub(in super::super) fn consume_pending_message_jump_target(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<String> {
        if self.is_context_preview(cx) {
            return None;
        }
        let presenter = ChatMessageJumpPresenter::shared(cx);
        let pending_names_topic = presenter.read(cx).pending().is_some_and(|pending| pending.topic_id.is_some());
        if pending_names_topic && !self.is_topics_mode(cx) && !self.has_resolved_chat_metadata(cx) {
            return None;
        }
        let chat_id = self.resolved_chat_id(cx);
        let thread_root_id = self.resolved_thread_root_id().map(str::to_owned);
        let request = presenter.update(cx, |presenter, cx| presenter.consume(&chat_id, thread_root_id.as_deref(), cx))?;

        if self.is_topics_mode(cx) {
            match request.topic_id.as_deref().filter(|topic_id| !topic_id.is_empty()) {
                Some(topic_id) if self.topic_is_unreadable(topic_id, cx) => {}
                Some(topic_id) => self.select_topic_tab(ChatTopicTab::Topic(topic_id.to_owned()), window, cx),
                None => {
                    let tab = if self.all_topics_view_hidden(cx) { ChatTopicTab::General } else { ChatTopicTab::All };
                    self.select_topic_tab(tab, window, cx);
                }
            }
        }
        self.delivered_jump_message_id = Some(request.message_id.clone());
        Some(request.message_id)
    }

    /// claims the list for a travel and arms its wait
    /// split from the request on purpose, the load task runs the travel in its own seed
    /// but the list must be parked and the clock started when the reader asks
    pub(in super::super) fn begin_message_jump(&mut self, window: &mut Window, cx: &mut Context<Self>) -> ChatMessageJumpToken {
        if !self.state.is_message_search_active {
            resign_chat_keyboard(window, cx);
        }
        let scope = self.message_scope(cx);
        message_interactions(cx, |store, cx| store.cancel_pending_latest_messages_jump(&scope, cx));
        self.pending_latest_jump_scope = None;
        self.pending_date_jump_id = None;
        let jump_id = Uuid::new_v4();
        self.pending_pinned_jump_id = Some(jump_id);
        self.message_list_scroll_request = Some(RiseMessageScrollRequest::new(RiseMessageScrollDestination::Takeover, false));
        let progress_scope = ChatMessageJumpProgressScope {
            chat_id: self.resolved_chat_id(cx),
            topic_id: self.resolved_topic_id(cx),
            thread_root_id: self.resolved_thread_root_id().map(str::to_owned),
        };
        let progress_token = ChatMessageJumpProgress::shared(cx).update(cx, |progress, cx| progress.begin(progress_scope, cx));
        cx.notify();
        ChatMessageJumpToken { jump_id, scope, progress_token }
    }

    /// ends a travel, list moves or it fails
    /// superseded travel reports nothing, newer one owns the list
    /// must clear the pending flag or next jump is dropped
    pub(in super::super) fn finish_message_jump(
        &mut self,
        token: &ChatMessageJumpToken,
        message_id: &str,
        outcome: RiseMessageJumpOutcome,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let progress_token = token.progress_token;
        ChatMessageJumpProgress::shared(cx).update(cx, |progress, cx| progress.finish(progress_token, outcome, cx));
        if token.scope != self.message_scope(cx) || self.pending_pinned_jump_id != Some(token.jump_id) {
            return;
        }
        self.pending_pinned_jump_id = None;
        if self.delivered_jump_message_id.as_deref() == Some(message_id) {
            self.delivered_jump_message_id = None;
        }
        cx.notify();
        if outcome != RiseMessageJumpOutcome::Installed {
            return;
        }
        self.message_list_scroll_request =
            Some(RiseMessageScrollRequest::new(RiseMessageScrollDestination::Message(message_id.to_owned()), true));
        let _ = selection_changed();
    }

    pub(in super::super) fn reopen_messages_after_abandoned_jump(&mut self, scope: &str, _window: &mut Window, cx: &mut Context<Self>) {
        let chat_id = self.resolved_chat_id(cx);
        if scope != self.message_scope(cx) || chat_id.is_empty() || self.is_pending_chat(cx) {
            return;
        }
        self.load_initial_messages(cx);
    }

    /// the one way an open screen travels to a message
    /// inside the rendered window it moves the list at once, else it fetches a window around it first
    pub(in super::super) fn jump_to_message_id(&mut self, raw_message_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let message_id = trimming_whitespaces_and_newlines(raw_message_id).to_owned();
        let chat_id = self.resolved_chat_id(cx);
        if chat_id.is_empty() || message_id.is_empty() {
            return;
        }

        if self.current_visible_messages(cx).iter().any(|message| message.matches_message_id(&message_id)) {
            let token = self.begin_message_jump(window, cx);
            self.finish_message_jump(&token, &message_id, RiseMessageJumpOutcome::Installed, window, cx);
            return;
        }
        if GetSaiServerId::is_temp_id(Some(message_id.as_str())) {
            return;
        }
        if let Some(tab) = self.topic_tab_admitting_loaded_message(&message_id, cx) {
            self.redeliver_message_jump(message_id, tab, window, cx);
            return;
        }

        let token = self.begin_message_jump(window, cx);
        let topic_id = self.resolved_topic_id(cx);
        let thread_root_id = self.resolved_thread_root_id().map(str::to_owned);
        let this = cx.weak_entity();
        let handle = window.window_handle();
        let anchor_message_id = message_id.clone();
        let completion: MessageJumpCompletion = Box::new(move |outcome, cx: &mut App| {
            Self::deliver_in_window(&this, handle, cx, move |screen, window, cx| {
                if outcome == RiseMessageJumpOutcome::Installed
                    && token.scope == screen.message_scope(cx)
                    && screen.pending_pinned_jump_id == Some(token.jump_id)
                    && let Some(tab) = screen.topic_tab_admitting_loaded_message(&message_id, cx)
                {
                    let progress_token = token.progress_token;
                    ChatMessageJumpProgress::shared(cx).update(cx, |progress, cx| progress.finish(progress_token, outcome, cx));
                    screen.redeliver_message_jump(message_id, tab, window, cx);
                    return;
                }
                screen.finish_message_jump(&token, &message_id, outcome, window, cx);
            });
        });
        message_interactions(cx, |store, cx| {
            store.load_messages_around(
                &chat_id,
                topic_id.as_deref(),
                thread_root_id.as_deref(),
                &anchor_message_id,
                Some(completion),
                cx,
            );
        });
    }

    pub(in super::super) fn handle_date_separator_tap(&mut self, day_start_ms: i64, is_pinned: bool, window: &mut Window, cx: &mut Context<Self>) {
        let chat_id = self.resolved_chat_id(cx);
        if chat_id.is_empty() {
            return;
        }
        resign_chat_keyboard(window, cx);
        if is_pinned {
            self.jump_to_day_start(day_start_ms, window, cx);
        } else {
            let topic_id = self.resolved_topic_id(cx);
            let thread_root_id = self.resolved_thread_root_id().map(str::to_owned);
            let calendar = Calendar::current();
            let locale = Locale::app_locale();
            let now = Date::now();
            message_interactions(cx, |store, cx| {
                store.open_chat_calendar(&chat_id, topic_id.as_deref(), thread_root_id.as_deref(), &calendar, &locale, &now, cx);
            });
            self.pending_chat_calendar_day_start_ms = None;
            self.pending_chat_calendar_day_installed = false;
            self.show_chat_calendar = true;
            cx.notify();
        }
    }

    pub(in super::super) fn handle_chat_calendar_day_tap(&mut self, day: RiseChatCalendarDay, window: &mut Window, cx: &mut Context<Self>) {
        let day_start_ms = day.day_start_ms;
        self.pending_chat_calendar_day_start_ms = Some(day_start_ms);
        self.pending_chat_calendar_day_installed = false;
        self.show_chat_calendar = false;
        let this = cx.weak_entity();
        let handle = window.window_handle();
        let completion: Rc<dyn Fn(bool, &mut App)> = Rc::new(move |installed: bool, cx: &mut App| {
            Self::deliver_in_window(&this, handle, cx, move |screen, window, cx| {
                if screen.pending_chat_calendar_day_start_ms != Some(day_start_ms) {
                    return;
                }
                screen.pending_chat_calendar_day_installed = installed;
                if !screen.show_chat_calendar {
                    screen.finish_pending_chat_calendar_jump(window, cx);
                }
            });
        });
        message_interactions(cx, |store, cx| store.select_chat_calendar_day(&day, Some(completion), cx));
        cx.notify();
    }

    /// idempotent, both the dismissal and the fetch call it and the first one consumes the pending day
    pub(in super::super) fn finish_pending_chat_calendar_jump(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(day_start_ms) = self.pending_chat_calendar_day_start_ms else {
            return;
        };
        if !self.pending_chat_calendar_day_installed {
            return;
        }
        self.pending_chat_calendar_day_start_ms = None;
        self.pending_chat_calendar_day_installed = false;
        self.message_list_scroll_request =
            Some(RiseMessageScrollRequest::new(RiseMessageScrollDestination::DayStart(day_start_ms), true));
        let _ = selection_changed();
        cx.notify();
    }

    pub(in super::super) fn jump_to_day_start(&mut self, day_start_ms: i64, _window: &mut Window, cx: &mut Context<Self>) {
        let chat_id = self.resolved_chat_id(cx);
        if chat_id.is_empty() {
            return;
        }
        let scope = self.message_scope(cx);
        let topic_id = self.resolved_topic_id(cx);
        let thread_root_id = self.resolved_thread_root_id().map(str::to_owned);

        message_interactions(cx, |store, cx| store.cancel_pending_latest_messages_jump(&scope, cx));
        self.pending_latest_jump_scope = None;
        ChatMessageJumpProgress::shared(cx).update(cx, |progress, cx| progress.abandon(cx));
        self.pending_pinned_jump_id = None;
        let jump_id = Uuid::new_v4();
        self.pending_date_jump_id = Some(jump_id);
        self.message_list_scroll_request = Some(RiseMessageScrollRequest::new(RiseMessageScrollDestination::Takeover, false));
        message_interactions(cx, |store, cx| {
            store.cancel_pending_message_navigation_request(&chat_id, topic_id.as_deref(), thread_root_id.as_deref(), cx);
        });

        let messages = self.current_visible_messages(cx);
        let created_ms =
            |message: &GetMessageMessage| RiseChatCalendarWindow::milliseconds(&Date::from_time_interval_since_1970(message.created_at));
        let is_loaded_locally =
            messages.iter().any(|message| created_ms(message) >= day_start_ms) && messages.iter().any(|message| created_ms(message) < day_start_ms);

        let this = cx.weak_entity();
        let completion: Rc<dyn Fn(bool, &mut App)> = Rc::new(move |installed: bool, cx: &mut App| {
            let requested_scope = scope.clone();
            Self::deliver(&this, cx, move |screen, cx| {
                if !installed {
                    if screen.pending_date_jump_id == Some(jump_id) {
                        screen.pending_date_jump_id = None;
                    }
                    return;
                }
                if requested_scope != screen.message_scope(cx) || screen.pending_date_jump_id != Some(jump_id) {
                    return;
                }
                screen.pending_date_jump_id = None;
                screen.message_list_scroll_request =
                    Some(RiseMessageScrollRequest::new(RiseMessageScrollDestination::DayStart(day_start_ms), true));
                let _ = selection_changed();
                cx.notify();
            });
        });
        message_interactions(cx, |store, cx| {
            store.jump_to_chat_calendar_day(
                &chat_id,
                topic_id.as_deref(),
                thread_root_id.as_deref(),
                day_start_ms,
                is_loaded_locally,
                Some(completion),
                cx,
            );
        });
        cx.notify();
    }

    pub(in super::super) fn jump_to_latest_messages(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.resolved_chat_id(cx).is_empty() {
            return;
        }
        self.travel_to_latest_messages(cx);
        let _ = selection_changed();
    }

    /// your own message is followed like in telegram
    /// sent from above the bottom it brings the newest page back
    /// a window that already holds the bottom glides there by itself
    pub(in super::super) fn follow_outgoing_message(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.is_context_preview(cx) || self.resolved_chat_id(cx).is_empty() || self.is_pending_chat(cx) {
            return;
        }
        let scope = self.message_scope(cx);
        let presentation = message_services(cx, |store, cx| store.message_scope_presentation_snapshot(&scope, cx));
        let has_cached_after = match presentation.as_ref() {
            Some(presentation) => presentation.slice.has_cached_after(),
            None => message_services(cx, |store, cx| store.message_presentation_slice(&scope, cx)).has_cached_after(),
        };
        let can_load_newer = has_cached_after
            || match presentation.as_ref() {
                Some(presentation) => presentation.has_more_after,
                None => message_services(cx, |store, cx| store.has_more_messages(&scope, MessageHistoryDirection::Newer, cx)),
            };
        if can_load_newer {
            self.travel_to_latest_messages(cx);
        }
    }

    pub(in super::super) fn record_latest_jump_trace(&self, event: &str, token: Option<u64>, detail: Option<&str>, cx: &mut App) {
        let scope = self.message_scope(cx);
        let cached_count = message_services(cx, |store, cx| store.cached_message_count(&scope, cx));
        MessagePaginationTrace::record(event, &scope, None, token, None, None, Some(cached_count), None, None, None, detail, cx);
    }

    pub(in super::super) fn travel_to_latest_messages(&mut self, cx: &mut Context<Self>) {
        let chat_id = self.resolved_chat_id(cx);
        if chat_id.is_empty() {
            return;
        }
        let topic_id = self.resolved_topic_id(cx);
        let thread_root_id = self.resolved_thread_root_id().map(str::to_owned);

        ChatMessageJumpProgress::shared(cx).update(cx, |progress, cx| progress.abandon(cx));
        self.pending_pinned_jump_id = None;
        message_interactions(cx, |store, cx| {
            store.cancel_pending_message_navigation_request(&chat_id, topic_id.as_deref(), thread_root_id.as_deref(), cx);
        });
        self.message_list_scroll_request = Some(RiseMessageScrollRequest::new(RiseMessageScrollDestination::Latest, true));
        let unread_detail = format!("unread={}", self.active_unread_count(cx).max(0));
        self.record_latest_jump_trace("jump.latest.tap", None, Some(unread_detail.as_str()), cx);
        let this = cx.weak_entity();
        let completion: MessageActionFlagHandler = Box::new(move |installed, cx: &mut App| {
            Self::deliver(&this, cx, move |screen, cx| {
                let scope = screen.message_scope(cx);
                tracing::warn!(
                    target: "riseonly::X2024",
                    installed,
                    pending = screen.pending_latest_jump_scope.as_deref() == Some(scope.as_str()),
                    "latest jump completion"
                );
                if screen.pending_latest_jump_scope.as_deref() != Some(scope.as_str()) {
                    return;
                }
                screen.pending_latest_jump_scope = None;
                if installed {
                    screen.finish_latest_messages_jump(cx);
                }
                cx.notify();
            });
        });
        let installed = message_interactions(cx, |store, cx| {
            store.jump_to_latest_messages(&chat_id, topic_id.as_deref(), thread_root_id.as_deref(), Some(completion), cx)
        });
        let install_detail = format!("installed={installed}");
        self.record_latest_jump_trace("jump.latest.install", None, Some(install_detail.as_str()), cx);
        if installed {
            self.pending_latest_jump_scope = None;
            self.finish_latest_messages_jump(cx);
        } else {
            self.pending_latest_jump_scope = Some(self.message_scope(cx));
        }
        if !self.is_context_preview(cx)
            && !self.is_message_request(cx)
            && !self.is_community_outsider(cx)
            && self.active_unread_count(cx) > 0
        {
            let read_topic_id = self.read_topic_id(cx);
            if let Some(task) = message_interactions(cx, |store, cx| {
                store.mark_chat_as_read(&chat_id, read_topic_id.as_deref(), false, cx)
            }) {
                task.detach();
            }
        }
        cx.notify();
    }

    pub(in super::super) fn complete_pending_latest_messages_jump(&mut self, revision: u64, _window: &mut Window, cx: &mut Context<Self>) {
        let scope = self.message_scope(cx);
        if self.pending_latest_jump_scope.as_deref() != Some(scope.as_str()) {
            return;
        }
        self.pending_latest_jump_scope = None;
        self.record_latest_jump_trace("jump.latest.complete", Some(revision), None, cx);
        self.finish_latest_messages_jump(cx);
        cx.notify();
    }

    pub(in super::super) fn finish_latest_messages_jump(&mut self, cx: &mut Context<Self>) {
        let already_latest = self
            .message_list_scroll_request
            .as_ref()
            .is_some_and(|request| matches!(request.destination, RiseMessageScrollDestination::Latest));
        if !already_latest {
            self.message_list_scroll_request = Some(RiseMessageScrollRequest::new(RiseMessageScrollDestination::Latest, true));
        }
        self.message_viewport_chrome.update(cx, |chrome, cx| chrome.finish_latest_jump(cx));
    }
}
