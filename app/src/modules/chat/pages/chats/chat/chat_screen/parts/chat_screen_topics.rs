use std::collections::HashMap;
use gpui::prelude::*;
use gpui::{AnyElement, App, AppContext, Context, Empty, Window, div};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use crate::app::router::navigation_types::RootRoute;
use crate::modules::chat::components::chat::composer::chat_composer_inline_context::ChatComposerInlineContext;
use crate::modules::chat::components::chat::composer::chat_composer_types::resign_chat_keyboard;
use crate::modules::chat::pages::chats::chat::chat_roles::topic_mention_composer::TopicMentionComposer;
use crate::modules::chat::pages::chats::chats_aside_hooks::chats_aside_hooks;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatPerm, ChatPermExt, ChatTopic, ChatType};
use crate::modules::chat::stores::chats::chat_services::chat_topics_services::ChatRemovedTopicNotice;
use super::super::chat_screen::*;
use super::super::chat_screen_support::*;

impl ChatScreen {
    pub(in super::super) fn resolved_topic_id(&self, cx: &mut App) -> Option<String> {
        if !self.is_topics_mode(cx) {
            return self.topic_id.clone();
        }
        match &self.state.selected_topic_tab {
            ChatTopicTab::All => None,
            ChatTopicTab::General => Some(String::new()),
            ChatTopicTab::Topic(id) => Some(id.clone()),
        }
    }

    pub(in super::super) fn is_topics_mode(&self, cx: &mut App) -> bool {
        if self.is_thread_screen() {
            return false;
        }
        if self.topic_id.is_some() || self.initial_all_view {
            return true;
        }
        let chat_id = self.resolved_chat_id(cx);
        chat_services(cx, |store, cx| store.topics_enabled(&chat_id, cx))
    }

    pub(in super::super) fn bar_topics(&self, cx: &mut App) -> Vec<ChatTopic> {
        let chat_id = self.resolved_chat_id(cx);
        let topics = chat_services(cx, |store, cx| store.topics(&chat_id, cx));
        topics.into_iter().filter(|topic| !topic.is_general).collect()
    }

    pub(in super::super) fn post_topic_id(&self, cx: &mut App) -> Option<String> {
        if !self.is_topics_mode(cx) {
            return self.resolved_topic_id(cx);
        }
        match &self.state.selected_topic_tab {
            ChatTopicTab::All | ChatTopicTab::General => None,
            ChatTopicTab::Topic(id) => Some(id.clone()),
        }
    }

    pub(in super::super) fn show_topic_bar(&self) -> bool {
        false
    }

    pub(in super::super) fn group_feed_by_topic(&self, cx: &mut App) -> bool {
        self.is_topics_mode(cx) && self.state.selected_topic_tab == ChatTopicTab::All
    }

    pub(in super::super) fn header_topic_id(&self, cx: &mut App) -> Option<String> {
        if !self.is_topics_mode(cx) {
            return None;
        }
        match &self.state.selected_topic_tab {
            ChatTopicTab::Topic(id) => Some(id.clone()),
            _ => None,
        }
    }

    pub(in super::super) fn header_topic_title(&self, cx: &mut App) -> Option<String> {
        if !self.is_topics_mode(cx) {
            return None;
        }
        match &self.state.selected_topic_tab {
            ChatTopicTab::Topic(id) => {
                let chat_id = self.resolved_chat_id(cx);
                chat_services(cx, |store, cx| store.topic_from_any_cache(&chat_id, id, cx)).map(|topic| topic.title)
            }
            ChatTopicTab::General => Some(tr("chat_topics_general")),
            ChatTopicTab::All => None,
        }
    }

    pub(in super::super) fn topic_titles_map(&self, cx: &mut App) -> HashMap<String, String> {
        if !self.is_topics_mode(cx) {
            return HashMap::new();
        }
        let chat_id = self.resolved_chat_id(cx);
        chat_services(cx, |store, cx| store.topics(&chat_id, cx))
            .into_iter()
            .map(|topic| (topic.id, topic.title))
            .collect()
    }

    pub(in super::super) fn read_topic_id(&self, cx: &mut App) -> Option<String> {
        if !self.is_thread_screen() {
            return self.resolved_topic_id(cx);
        }
        let requested = trimming_whitespaces_and_newlines(self.topic_id.as_deref().unwrap_or(""));
        if !requested.is_empty() {
            return Some(requested.to_owned());
        }
        let chat_id = self.resolved_chat_id(cx);
        chat_services(cx, |store, cx| store.topics_enabled(&chat_id, cx)).then(String::new)
    }

    pub(in super::super) fn inline_composer_topic_id(&self, cx: &mut App) -> Option<String> {
        self.post_topic_id(cx).or_else(|| self.resolved_topic_id(cx))
    }

    pub(in super::super) fn inline_composer_topics(&self, cx: &mut App) -> Vec<ChatTopic> {
        let ChatComposerInlineContext::Topics { query, .. } = &self.inline_composer_context else {
            return Vec::new();
        };
        let chat_id = self.resolved_chat_id(cx);
        TopicMentionComposer::matches(query, &chat_id, cx)
    }

    pub(in super::super) fn inline_composer_allows_topic_mentions(&self, cx: &mut App) -> bool {
        if self.resolved_chat(cx).r#type != ChatType::Group {
            return false;
        }
        let chat_id = self.resolved_chat_id(cx);
        let topic_id = self.inline_composer_topic_id(cx);
        chat_services(cx, |store, cx| store.topics_enabled(&chat_id, cx))
            && TopicMentionComposer::can_mention(&chat_id, topic_id.as_deref(), cx)
    }

    pub(in super::super) fn topic_posting_locked(&self, cx: &mut App) -> bool {
        if !self.is_topics_mode(cx) {
            return false;
        }
        let ChatTopicTab::Topic(id) = &self.state.selected_topic_tab else {
            return false;
        };
        let chat_id = self.resolved_chat_id(cx);
        !chat_services(cx, |store, cx| store.can_post_in_topic(&chat_id, id, cx))
    }

    pub(in super::super) fn composer_topic_mask_load_key(&self, cx: &mut App) -> String {
        if !self.has_resolved_chat_metadata(cx) {
            return String::new();
        }
        let chat = self.resolved_chat(cx);
        if !chat.topics_enabled || ChatPerm::is_owner_or_legacy_admin_role(chat.role.as_deref()) {
            return String::new();
        }
        let Some(topic_id) = self.post_topic_id(cx).filter(|topic_id| !topic_id.is_empty()) else {
            return String::new();
        };
        let chat_id = self.resolved_chat_id(cx);
        if chat_services(cx, |store, cx| store.memory_only_permissions(&chat_id, Some(&topic_id), cx)).is_some() {
            return String::new();
        }
        format!("{chat_id}|{topic_id}")
    }

    /// tab a forum must be read under for a message to be in the list
    /// none when the current tab already has it
    /// cross topic travel must switch the history scope before requesting
    pub(in super::super) fn topic_tab_admitting(&self, topic_id: Option<&str>, cx: &mut App) -> Option<ChatTopicTab> {
        if !self.is_topics_mode(cx) {
            return None;
        }
        let topic = topic_id.map(trimming_whitespaces_and_newlines).unwrap_or_default().to_owned();
        if self.topic_is_unreadable(&topic, cx) {
            return None;
        }
        match &self.state.selected_topic_tab {
            ChatTopicTab::All => None,
            ChatTopicTab::General => (!topic.is_empty()).then(|| ChatTopicTab::Topic(topic)),
            ChatTopicTab::Topic(current) => {
                if &topic == current {
                    None
                } else if topic.is_empty() {
                    Some(ChatTopicTab::General)
                } else {
                    Some(ChatTopicTab::Topic(topic))
                }
            }
        }
    }

    /// true for a private topic this account cant read, so the tab doesnt move there
    /// a topic the forum doesnt list keeps the current tab and the jump fails in place with a toast
    pub(in super::super) fn topic_is_unreadable(&self, topic_id: &str, cx: &mut App) -> bool {
        if topic_id.is_empty() {
            return false;
        }
        let chat_id = self.resolved_chat_id(cx);
        chat_services(cx, |store, cx| store.in_memory_forum_readable_topic_ids(&chat_id, cx))
            .is_some_and(|readable| !readable.contains(topic_id))
    }

    /// aggregate feed and pin index can know a target outside the selected topic window
    /// an absent message is unknown and not general so it must not move the tab
    pub(in super::super) fn topic_tab_admitting_loaded_message(&self, message_id: &str, cx: &mut App) -> Option<ChatTopicTab> {
        if !self.is_topics_mode(cx) {
            return None;
        }
        let scope = self.message_scope(cx);
        let chat_id = self.resolved_chat_id(cx);
        let loaded = message_services(cx, |store, cx| {
            store
                .message(message_id, &scope, cx)
                .or_else(|| store.message(message_id, &chat_id, cx))
                .or_else(|| store.pinned_messages(&chat_id, cx).into_iter().find(|message| message.matches_message_id(message_id)))
        })?;
        self.topic_tab_admitting(loaded.topic_id.as_deref(), cx)
    }

    pub(in super::super) fn topic_composer_placeholder(&self, cx: &mut App) -> String {
        let title = match &self.state.selected_topic_tab {
            ChatTopicTab::All | ChatTopicTab::General => tr("chat_topics_general"),
            ChatTopicTab::Topic(id) => {
                let chat_id = self.resolved_chat_id(cx);
                chat_services(cx, |store, cx| store.topic_from_any_cache(&chat_id, id, cx))
                    .map(|topic| topic.title)
                    .unwrap_or_else(|| tr("chat_topics_topic"))
            }
        };
        tr("chat_topic_composer_placeholder").replace("{topic}", &title)
    }

    pub(in super::super) fn select_inline_topic(&mut self, topic: &ChatTopic, window: &mut Window, cx: &mut Context<Self>) {
        let ChatComposerInlineContext::Topics { token_range, .. } = self.inline_composer_context.clone() else {
            return;
        };
        let (text, selection) =
            ChatComposerInlineContext::replacing(token_range, &self.state.input_text, &format!("#{} ", topic.title));
        self.set_composer_text(text, selection, cx);
        self.dismiss_inline_composer(false, window, cx);
    }

    pub(in super::super) fn all_topics_view_hidden(&self, cx: &mut App) -> bool {
        let chat_id = self.resolved_chat_id(cx);
        chat_services(cx, |store, cx| store.all_topics_view_hidden(&chat_id, cx))
    }

    pub(in super::super) fn leave_hidden_all_topics_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_topics_mode(cx)
            || self.is_thread_screen()
            || !self.all_topics_view_hidden(cx)
            || self.state.selected_topic_tab != ChatTopicTab::All
        {
            return;
        }
        self.select_topic_tab(ChatTopicTab::General, window, cx);
    }

    pub(in super::super) fn leave_removed_topic_tab(&mut self, notice: Option<&ChatRemovedTopicNotice>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(notice) = notice else {
            return;
        };
        if !self.is_topics_mode(cx) || self.is_thread_screen() || notice.chat_id != self.resolved_chat_id(cx) {
            return;
        }
        let ChatTopicTab::Topic(selected) = &self.state.selected_topic_tab else {
            return;
        };
        if !notice.topic_ids.contains(selected) {
            return;
        }
        if self.topic_id.as_ref().is_some_and(|topic_id| notice.topic_ids.contains(topic_id)) {
            return;
        }
        self.select_topic_tab(ChatTopicTab::All, window, cx);
        self.leave_hidden_all_topics_view(window, cx);
        notice.announce_once(cx);
    }

    pub(in super::super) fn select_topic_tab(&mut self, tab: ChatTopicTab, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.selected_topic_tab == tab && self.state.topic_selection_initialized {
            return;
        }
        if self.state.edit_target.is_some() {
            self.cancel_edit(window, cx);
        }
        tracing::warn!(target: "riseonly::X1191", from = ?self.state.selected_topic_tab, to = ?tab, "topic tab select");
        resign_chat_keyboard(window, cx);
        self.inline_invocation_exclusion_tag = None;
        self.dismiss_inline_composer(false, window, cx);
        let chat_id = self.resolved_chat_id(cx);
        let open_info = match &tab {
            ChatTopicTab::Topic(topic_id) => {
                Some(RootRoute::TopicProfile { chat_id: Some(chat_id.clone()), topic_id: Some(topic_id.clone()) })
            }
            ChatTopicTab::General => Some(RootRoute::GeneralProfile { chat_id: Some(chat_id.clone()) }),
            ChatTopicTab::All => None,
        };
        if let Some(hooks) = chats_aside_hooks() {
            (hooks.follow_topic)(&chat_id, open_info.as_ref(), cx);
        }
        self.state.selected_topic_tab = tab;
        self.state.topic_selection_initialized = true;
        self.state.reply_target = None;
        self.state.edit_target = None;
        cx.notify();
    }

    pub(in super::super) fn topic_locked_bar(&mut self, theme: &AppTheme, safe_bottom_inset: f32, cx: &mut Context<Self>) -> AnyElement {
        let pill = div()
            .id("chat-topic-locked-host")
            .tooltip_label(tr("chat_topic_closed_banner"))
            .child(Self::locked_pill("lock.fill", tr("chat_topic_closed_pill"), theme))
            .into_any_element();
        self.composer_plate_row(pill, safe_bottom_inset, cx)
    }
}
