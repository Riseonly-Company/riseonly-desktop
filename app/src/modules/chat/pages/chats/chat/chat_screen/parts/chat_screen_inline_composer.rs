use std::rc::Rc;
use gpui::prelude::*;
use gpui::{App, AppContext, Context, Window};
use rise_canon::RiseGuideSignal;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::ns_range::NSRange;
use rise_i18n::Locale;
use rise_ui::swiftui::prelude::*;
use crate::modules::bot::stores::bot::bot_actions::bot_inline_types::{BotInlineCommandsModel, BotInlineIdentityModel, BotInlineResultModel, BotInlineResultsPage, BotMentionCandidateModel};
use crate::modules::bot::stores::bot::bot_actions::bot_group_command_types::{ChatGroupBotCommandModel, ChatGroupBotCommands};
use crate::modules::bot::stores::bot::bot_actions::bot_types::BotCommandModel;
use crate::modules::bot::stores::bot::bot_services::bot_services::BotServicesStore;
use crate::modules::chat::components::chat::composer::chat_composer_inline_context::ChatComposerMentionEntityCodec;
use crate::modules::chat::components::chat::composer::chat_inline_composer_panel::ChatInlineComposerPanel;
use rise_bot_api::ChatAction;
use crate::modules::chat::components::chat::composer::chat_composer_inline_context::ChatComposerInlineContext;
use crate::modules::chat::components::chat::composer::chat_composer_types::{ChatBottomBarMetrics, ChatComposerUserMention};
use crate::modules::chat::components::chat::composer::chat_inline_composer_panel::ChatInlineComposerPresentationContract;
use crate::modules::chat::pages::chats::chat::chat_roles::role_mention_composer::RoleMentionComposer;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatRole, ChatTopic, ChatType};
use rise_ui::swiftui::button_style::plain;
use crate::modules::chat::components::chat::scroll::chat_messages_scroll_controller::ChatComposerInsetAnimation;
use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::modules::bot::stores::bot::bot_actions::bot_actions::BotInlineSendResult;
use super::super::chat_screen::*;
use super::super::chat_screen_support::*;

impl ChatScreen {
    pub(in super::super) fn inline_composer_scope(&self, cx: &mut App) -> String {
        format!("inline-composer::{}", self.message_scope(cx))
    }

    pub(in super::super) fn inline_composer_language_code(&self) -> String {
        let locale = Locale::app_locale();
        let code = locale.identifier().split(['-', '_']).next().unwrap_or("");
        if code.is_empty() { "en".to_owned() } else { code.to_owned() }
    }

    pub(in super::super) fn inline_composer_control_mode_active(&self, peer_blocked: bool, cx: &mut App) -> bool {
        ChatInlineComposerPresentationContract::inline_controls_are_active(
            self.inline_composer_has_visible_content(cx),
            self.state.edit_target.is_none()
                && !self.is_message_request(cx)
                && !self.state.is_message_selection_active
                && !self.state.is_message_search_active
                && !self.topic_posting_locked(cx)
                && !self.channel_posting_locked(cx)
                && !self.composer_text_locked(cx)
                && !peer_blocked
                && !self.is_any_capture_active(cx)
                && !self.show_bot_chat_menu,
        )
    }

    pub(in super::super) fn inline_composer_has_visible_content(&self, cx: &mut App) -> bool {
        match &self.inline_composer_context {
            ChatComposerInlineContext::Inactive => false,
            ChatComposerInlineContext::Candidates { .. } => {
                if !self.inline_composer_candidates(cx).is_empty() || !self.inline_composer_roles(cx).is_empty() {
                    return true;
                }
                let Some(query) = self.inline_composer_candidates_query(cx) else {
                    return false;
                };
                let chat_id = self.resolved_chat_id(cx);
                let topic_id = self.inline_composer_topic_id(cx);
                BotServicesStore::has_mention_candidates(&chat_id, topic_id.as_deref(), &query, cx)
                    || self.inline_composer_panel_error(cx).is_some()
                    || self.inline_composer_panel_loading(cx)
            }
            ChatComposerInlineContext::Bot { .. } => true,
            ChatComposerInlineContext::GroupCommands { .. } => !self.inline_composer_group_commands(cx).is_empty(),
            ChatComposerInlineContext::Topics { .. } => !self.inline_composer_topics(cx).is_empty(),
        }
    }

    /// commands of the bots in this group that match the typed slash query
    pub(in super::super) fn inline_composer_group_commands(&self, cx: &mut App) -> Vec<ChatGroupBotCommandModel> {
        let ChatComposerInlineContext::GroupCommands { query, .. } = &self.inline_composer_context else {
            return Vec::new();
        };
        let chat_id = self.resolved_chat_id(cx);
        let topic_id = self.inline_composer_topic_id(cx);
        let Some(groups) =
            BotServicesStore::chat_commands(&chat_id, topic_id.as_deref(), &self.inline_composer_language_code(), cx)
        else {
            return Vec::new();
        };
        ChatGroupBotCommands::filter(&ChatGroupBotCommands::flatten(&groups), query)
    }

    pub(in super::super) fn inline_composer_allows_group_commands(&self, cx: &mut App) -> bool {
        self.resolved_chat(cx).r#type == ChatType::Group
    }

    pub(in super::super) fn inline_composer_panel_detached(&self) -> bool {
        match &self.inline_composer_context {
            ChatComposerInlineContext::Bot { query, .. } => !is_blank(query),
            _ => false,
        }
    }

    pub(in super::super) fn inline_composer_panel_visible(&self, peer_blocked: bool, cx: &mut App) -> bool {
        ChatInlineComposerPresentationContract::panel_is_visible(
            self.inline_composer_control_mode_active(peer_blocked, cx),
            self.picker_visible,
        )
    }

    pub(in super::super) fn inline_composer_candidates(&self, cx: &mut App) -> Vec<BotMentionCandidateModel> {
        let Some(query) = self.inline_composer_candidates_query(cx) else {
            return Vec::new();
        };
        let chat_id = self.resolved_chat_id(cx);
        let topic_id = self.inline_composer_topic_id(cx);
        BotServicesStore::mention_candidates(&chat_id, topic_id.as_deref(), &query, cx)
    }

    /// while the typed query loads the panel keeps the last answered one on screen
    /// so it doesnt close and reopen on every keystroke
    /// a failed query shows its own error and retry
    pub(in super::super) fn inline_composer_candidates_query(&self, cx: &mut App) -> Option<String> {
        let ChatComposerInlineContext::Candidates { query, .. } = &self.inline_composer_context else {
            return None;
        };
        let Some(answered) = self.inline_candidates_answered_query.as_ref().filter(|answered| *answered != query) else {
            return Some(query.clone());
        };
        let chat_id = self.resolved_chat_id(cx);
        let topic_id = self.inline_composer_topic_id(cx);
        if BotServicesStore::has_mention_candidates(&chat_id, topic_id.as_deref(), query, cx) {
            return Some(query.clone());
        }
        let cache_key = self.inline_composer_cache_key(cx);
        if BotServicesStore::shared(cx).read(cx).inline_composer_error(&cache_key).is_some() {
            return Some(query.clone());
        }
        Some(answered.clone())
    }

    pub(in super::super) fn inline_composer_shows_answered_candidates(&self, cx: &mut App) -> bool {
        let ChatComposerInlineContext::Candidates { query, .. } = &self.inline_composer_context else {
            return false;
        };
        self.inline_composer_candidates_query(cx).as_deref() != Some(query.as_str())
    }

    pub(in super::super) fn inline_composer_panel_loading(&self, cx: &mut App) -> bool {
        if self.inline_composer_shows_answered_candidates(cx) {
            return false;
        }
        let cache_key = self.inline_composer_cache_key(cx);
        BotServicesStore::shared(cx).read(cx).is_inline_composer_loading(&cache_key)
    }

    pub(in super::super) fn inline_composer_panel_error(&self, cx: &mut App) -> Option<String> {
        if self.inline_composer_shows_answered_candidates(cx) {
            return None;
        }
        let cache_key = self.inline_composer_cache_key(cx);
        BotServicesStore::shared(cx).read(cx).inline_composer_error(&cache_key)
    }

    pub(in super::super) fn composer_offers_roles(&self, cx: &mut App) -> bool {
        !self.is_context_preview(cx)
            && self.inline_composer_has_visible_content(cx)
            && !self.inline_composer_roles(cx).is_empty()
    }

    pub(in super::super) fn inline_composer_roles(&self, cx: &mut App) -> Vec<ChatRole> {
        let ChatComposerInlineContext::Candidates { query, .. } = &self.inline_composer_context else {
            return Vec::new();
        };
        let kind = self.resolved_chat(cx).r#type;
        if kind != ChatType::Group && kind != ChatType::Channel {
            return Vec::new();
        }
        let chat_id = self.resolved_chat_id(cx);
        RoleMentionComposer::matches(query, &chat_id, cx)
    }

    pub(in super::super) fn inline_composer_commands(&self, cx: &mut App) -> Option<BotInlineCommandsModel> {
        let ChatComposerInlineContext::Bot { bot_tag, query, .. } = &self.inline_composer_context else {
            return None;
        };
        if !is_blank(query) {
            return None;
        }
        let chat_id = self.resolved_chat_id(cx);
        let topic_id = self.inline_composer_topic_id(cx);
        BotServicesStore::inline_commands(bot_tag, &chat_id, topic_id.as_deref(), &self.inline_composer_language_code(), cx)
    }

    pub(in super::super) fn empty_inline_results_page() -> BotInlineResultsPage {
        BotInlineResultsPage {
            query_id: String::new(),
            results: Vec::new(),
            next_offset: None,
            has_more: false,
            cache_time: 0,
        }
    }

    pub(in super::super) fn inline_composer_results(&self, cx: &mut App) -> BotInlineResultsPage {
        let ChatComposerInlineContext::Bot { bot_tag, query, .. } = &self.inline_composer_context else {
            return Self::empty_inline_results_page();
        };
        if is_blank(query) || ChatComposerInlineContext::command_awaiting_argument(query).is_some() {
            return Self::empty_inline_results_page();
        }
        let chat_id = self.resolved_chat_id(cx);
        let topic_id = self.inline_composer_topic_id(cx);
        BotServicesStore::inline_results(bot_tag, &chat_id, topic_id.as_deref(), query, cx)
    }

    pub(in super::super) fn inline_composer_cache_key(&self, cx: &mut App) -> String {
        let chat_id = self.resolved_chat_id(cx);
        let topic_id = self.inline_composer_topic_id(cx);
        let topic_id = topic_id.as_deref();
        match &self.inline_composer_context {
            ChatComposerInlineContext::Inactive => String::new(),
            ChatComposerInlineContext::Candidates { query, .. } => {
                BotServicesStore::mention_candidates_cache_key(&chat_id, topic_id, query, cx)
            }
            ChatComposerInlineContext::Bot { bot_tag, query, .. } => {
                if is_blank(query) {
                    return BotServicesStore::inline_commands_cache_key(
                        bot_tag,
                        &chat_id,
                        topic_id,
                        &self.inline_composer_language_code(),
                        cx,
                    );
                }
                if ChatComposerInlineContext::command_awaiting_argument(query).is_some() {
                    return String::new();
                }
                BotServicesStore::inline_results_cache_key(bot_tag, &chat_id, topic_id, query, cx)
            }
            ChatComposerInlineContext::GroupCommands { .. } => {
                BotServicesStore::chat_commands_cache_key(&chat_id, topic_id, &self.inline_composer_language_code(), cx)
            }
            ChatComposerInlineContext::Topics { .. } => String::new(),
        }
    }

    pub(in super::super) fn inline_composer_candidates_has_more(&self, cx: &mut App) -> bool {
        let ChatComposerInlineContext::Candidates { query, .. } = &self.inline_composer_context else {
            return false;
        };
        let chat_id = self.resolved_chat_id(cx);
        let topic_id = self.inline_composer_topic_id(cx);
        BotServicesStore::mention_candidates_has_more(&chat_id, topic_id.as_deref(), query, cx)
    }

    pub(in super::super) fn inline_composer_candidates_cursor(&self, cx: &mut App) -> Option<String> {
        let ChatComposerInlineContext::Candidates { query, .. } = &self.inline_composer_context else {
            return None;
        };
        let chat_id = self.resolved_chat_id(cx);
        let topic_id = self.inline_composer_topic_id(cx);
        BotServicesStore::mention_candidates_cursor(&chat_id, topic_id.as_deref(), query, cx)
    }

    pub(in super::super) fn inline_composer_suggestion_panel(
        &mut self,
        available_height: f32,
        collapsed_height: f32,
        cx: &mut Context<Self>,
    ) -> ChatInlineComposerPanel {
        let this = cx.weak_entity();
        let results = self.inline_composer_results(cx);
        let fallback_query_id = results.query_id.clone();
        let entity = cx.entity();
        let bots = BotServicesStore::shared(cx);
        ChatInlineComposerPanel {
            id: "chat-inline-composer-panel".into(),
            context: self.inline_composer_context.clone(),
            candidates: self.inline_composer_candidates(cx),
            roles: self.inline_composer_roles(cx),
            commands: self.inline_composer_commands(cx),
            results,
            is_loading: self.inline_composer_panel_loading(cx),
            error: self.inline_composer_panel_error(cx),
            available_height,
            collapsed_height,
            pagination_cache_key: self.inline_composer_cache_key(cx),
            candidates_has_more: self.inline_composer_candidates_has_more(cx),
            candidates_cursor: self.inline_composer_candidates_cursor(cx),
            is_result_sending: Rc::new(move |result: &BotInlineResultModel, cx: &App| {
                let query_id = result.effective_query_id(&fallback_query_id);
                bots.read(cx).is_inline_result_sending(&query_id, &result.id)
            }),
            on_candidate: Rc::new({
                let this = this.clone();
                move |candidate: &BotMentionCandidateModel, window: &mut Window, cx: &mut App| {
                    let candidate = candidate.clone();
                    let _ = this.update(cx, |screen, cx| screen.select_inline_candidate(&candidate, window, cx));
                }
            }),
            on_role: Rc::new({
                let this = this.clone();
                move |role: &ChatRole, window: &mut Window, cx: &mut App| {
                    let role = role.clone();
                    let _ = this.update(cx, |screen, cx| screen.select_inline_role(&role, window, cx));
                }
            }),
            on_command: Rc::new({
                let this = this.clone();
                move |command: &BotCommandModel, window: &mut Window, cx: &mut App| {
                    let command = command.clone();
                    let _ = this.update(cx, |screen, cx| screen.select_inline_command(&command, window, cx));
                }
            }),
            on_result: Rc::new({
                let this = this.clone();
                move |result: &BotInlineResultModel, window: &mut Window, cx: &mut App| {
                    let result = result.clone();
                    let _ = this.update(cx, |screen, cx| screen.send_inline_result(&result, window, cx));
                }
            }),
            on_retry: Rc::new(Self::act(this.clone(), |screen, window, cx| screen.retry_inline_composer(window, cx))),
            on_load_more_candidates: Rc::new(Self::act(this.clone(), |screen, _, cx| screen.load_more_inline_candidates(cx))),
            on_load_more_results: Rc::new(Self::act(this.clone(), |screen, _, cx| screen.load_more_inline_results(cx))),
            on_height_change: Rc::new({
                let this = this.clone();
                move |height: Option<f32>, _: &mut Window, cx: &mut App| {
                    let _ = this.update(cx, |screen, cx| {
                        screen.inline_composer_interactive_height = height;
                        cx.notify();
                    });
                }
            }),
            group_commands: self.inline_composer_group_commands(cx),
            on_group_command: Rc::new({
                let this = this.clone();
                move |item: &ChatGroupBotCommandModel, window: &mut Window, cx: &mut App| {
                    let item = item.clone();
                    let _ = this.update(cx, |screen, cx| screen.select_inline_group_command(&item, window, cx));
                }
            }),
            topics: self.inline_composer_topics(cx),
            on_topic: Rc::new({
                let this = this.clone();
                move |topic: &ChatTopic, window: &mut Window, cx: &mut App| {
                    let topic = topic.clone();
                    let _ = this.update(cx, |screen, cx| screen.select_inline_topic(&topic, window, cx));
                }
            }),
            on_dismiss: Rc::new(Self::act(this, |screen, window, cx| screen.dismiss_inline_composer(true, window, cx))),
            is_detached: self.inline_composer_panel_detached(),
            is_expanded: Self::flag_binding(
                entity,
                |screen| screen.inline_composer_expanded,
                |screen, value| screen.inline_composer_expanded = value,
            ),
        }
    }

    pub(in super::super) fn refresh_inline_composer_context(&mut self, text: &str, _window: &mut Window, cx: &mut Context<Self>) {
        let source_length = NSRange::full(text).length;
        let selection = if self.composer_selection_text != text
            || self.composer_selection.location.saturating_add(self.composer_selection.length) > source_length
        {
            NSRange::new(source_length, 0)
        } else {
            self.composer_selection
        };

        if let Some(excluded) = self.inline_invocation_exclusion_tag.as_deref() {
            let prefix = format!("@{excluded}");
            let lowered = text.to_lowercase();
            if lowered != prefix && !lowered.starts_with(&format!("{prefix} ")) {
                self.inline_invocation_exclusion_tag = None;
            }
        }

        let allows_group_commands = self.inline_composer_allows_group_commands(cx);
        let allows_topic_mentions = self.inline_composer_allows_topic_mentions(cx);
        let mut next = ChatComposerInlineContext::resolve(
            text,
            Some(selection),
            &self.inline_user_mentions,
            allows_group_commands,
            allows_topic_mentions,
        );
        if let ChatComposerInlineContext::Bot { bot_tag, .. } = &next
            && self.inline_invocation_exclusion_tag.as_deref() == Some(bot_tag.as_str())
        {
            next = ChatComposerInlineContext::Inactive;
        }
        if self.state.edit_target.is_some() || self.is_message_request(cx) || self.topic_posting_locked(cx) {
            next = ChatComposerInlineContext::Inactive;
        }

        if let ChatComposerInlineContext::Bot { bot_tag, .. } = &next {
            let same_bot = self
                .inline_selected_bot_identity
                .as_ref()
                .is_some_and(|identity| identity.tag.to_lowercase() == bot_tag.to_lowercase());
            if !same_bot {
                self.inline_selected_bot_identity = None;
            }
        } else {
            self.inline_selected_bot_identity = None;
        }

        let previous = self.inline_composer_context.clone();
        if next == previous {
            return;
        }
        let should_collapse = Self::inline_composer_mode_key(&previous) != Self::inline_composer_mode_key(&next);

        let chat_id = self.resolved_chat_id(cx);
        let topic_id = self.inline_composer_topic_id(cx);
        if let (ChatComposerInlineContext::Candidates { .. }, ChatComposerInlineContext::Candidates { query: previous_query, .. }) =
            (&next, &previous)
        {
            if BotServicesStore::has_mention_candidates(&chat_id, topic_id.as_deref(), previous_query, cx) {
                self.inline_candidates_answered_query = Some(previous_query.clone());
            }
        } else {
            self.inline_candidates_answered_query = None;
        }

        let scope = self.inline_composer_scope(cx);
        bot_interactions(cx, |store, cx| store.cancel_inline_composer(&scope, cx));
        ChatComposerInsetAnimation::set_last_duration(ChatBottomBarMetrics::COMPOSER_ANIM_DURATION);
        self.inline_composer_context = next.clone();
        if should_collapse || !next.is_presented() {
            self.inline_composer_expanded = false;
            self.inline_composer_interactive_height = None;
        }
        if should_collapse
            && let ChatComposerInlineContext::Bot { bot_tag, query, .. } = &next
            && !is_blank(query)
            && self.inline_composer_bot_identity(cx).is_none()
        {
            let language_code = self.inline_composer_language_code();
            bot_interactions(cx, |store, cx| {
                store.load_inline_commands(bot_tag, &chat_id, topic_id.as_deref(), &language_code, &scope, cx);
            });
        }
        if matches!(next, ChatComposerInlineContext::GroupCommands { .. })
            && !should_collapse
            && BotServicesStore::chat_commands(&chat_id, topic_id.as_deref(), &self.inline_composer_language_code(), cx)
                .is_some()
        {
            cx.notify();
            return;
        }
        self.request_inline_composer_content(&next, true, cx);
        cx.notify();
    }

    pub(in super::super) fn inline_composer_mode_key(context: &ChatComposerInlineContext) -> String {
        match context {
            ChatComposerInlineContext::Inactive => "inactive".to_owned(),
            ChatComposerInlineContext::Candidates { .. } => "candidates".to_owned(),
            ChatComposerInlineContext::Bot { bot_tag, .. } => format!("bot::{bot_tag}"),
            ChatComposerInlineContext::GroupCommands { .. } => "groupCommands".to_owned(),
            ChatComposerInlineContext::Topics { .. } => "topics".to_owned(),
        }
    }

    pub(in super::super) fn request_inline_composer_content(&mut self, context: &ChatComposerInlineContext, debounced: bool, cx: &mut Context<Self>) {
        let chat_id = self.resolved_chat_id(cx);
        let topic_id = self.inline_composer_topic_id(cx);
        let scope = self.inline_composer_scope(cx);
        match context {
            ChatComposerInlineContext::Inactive => {}
            ChatComposerInlineContext::Candidates { query, .. } => {
                bot_interactions(cx, |store, cx| {
                    store.load_mention_candidates(&chat_id, topic_id.as_deref(), query, &scope, debounced && !query.is_empty(), cx);
                });
            }
            ChatComposerInlineContext::Bot { bot_tag, query, .. } => {
                if is_blank(query) {
                    let language_code = self.inline_composer_language_code();
                    bot_interactions(cx, |store, cx| {
                        store.load_inline_commands(bot_tag, &chat_id, topic_id.as_deref(), &language_code, &scope, cx);
                    });
                } else if ChatComposerInlineContext::command_awaiting_argument(query).is_none() {
                    bot_interactions(cx, |store, cx| {
                        store.query_inline_results(bot_tag, &chat_id, topic_id.as_deref(), query, &scope, debounced, cx);
                    });
                }
            }
            ChatComposerInlineContext::GroupCommands { .. } => {
                let language_code = self.inline_composer_language_code();
                bot_interactions(cx, |store, cx| {
                    store.load_chat_commands(&chat_id, topic_id.as_deref(), &language_code, &scope, cx);
                });
            }
            ChatComposerInlineContext::Topics { .. } => {}
        }
    }

    pub(in super::super) fn select_inline_candidate(&mut self, candidate: &BotMentionCandidateModel, window: &mut Window, cx: &mut Context<Self>) {
        let ChatComposerInlineContext::Candidates { token_range, .. } = self.inline_composer_context.clone() else {
            return;
        };
        let tag = candidate.tag.trim_matches('@').to_owned();
        if tag.is_empty() {
            return;
        }

        let (replacement_text, replacement_selection) =
            ChatComposerInlineContext::replacing(token_range, &self.state.input_text, &format!("@{tag} "));

        let invokes_inline_bot = candidate.is_inline_bot() && token_range.location == 0;
        if invokes_inline_bot {
            self.inline_invocation_exclusion_tag = None;
            self.inline_selected_bot_identity = Some(BotInlineIdentityModel {
                id: candidate.bot_id.clone(),
                bot_user_id: candidate.user_id.clone(),
                tag: tag.clone(),
                name: candidate.name.clone(),
                avatar_url: candidate.avatar_url.clone(),
                inline_placeholder: candidate.inline_placeholder.clone(),
            });
        } else {
            if token_range.location == 0 {
                self.inline_invocation_exclusion_tag = Some(tag.to_lowercase());
            }
            if !candidate.user_id.is_empty() {
                let mentions_in_current_text = ChatComposerMentionEntityCodec::rebased(
                    &self.inline_user_mentions,
                    &self.inline_user_mentions_anchor_text,
                    &self.state.input_text,
                );
                let mut updated_mentions =
                    ChatComposerMentionEntityCodec::rebased(&mentions_in_current_text, &self.state.input_text, &replacement_text);
                let mention_range = NSRange::new(token_range.location, NSRange::full(&format!("@{tag}")).length);
                updated_mentions
                    .retain(|mention| mention.range.intersection(mention_range).is_none_or(|overlap| overlap.length == 0));
                updated_mentions.push(ChatComposerUserMention {
                    user_id: candidate.user_id.clone(),
                    tag: tag.clone(),
                    range: mention_range,
                });
                self.inline_user_mentions = updated_mentions;
                self.inline_user_mentions_anchor_text = replacement_text.clone();
            }
        }
        self.set_composer_text(replacement_text, replacement_selection, cx);
        if !invokes_inline_bot {
            self.dismiss_inline_composer(false, window, cx);
        }
    }

    pub(in super::super) fn select_inline_role(&mut self, role: &ChatRole, window: &mut Window, cx: &mut Context<Self>) {
        let ChatComposerInlineContext::Candidates { token_range, .. } = self.inline_composer_context.clone() else {
            return;
        };
        if token_range.location == 0 {
            self.inline_invocation_exclusion_tag = Some(role.name.to_lowercase());
        }
        let (text, selection) =
            ChatComposerInlineContext::replacing(token_range, &self.state.input_text, &format!("@{} ", role.name));
        self.set_composer_text(text, selection, cx);
        self.dismiss_inline_composer(false, window, cx);
        GuideSignalBus::emit(RiseGuideSignal::RoleMentionInserted);
    }

    pub(in super::super) fn select_inline_command(&mut self, command: &BotCommandModel, _window: &mut Window, cx: &mut Context<Self>) {
        let ChatComposerInlineContext::Bot { query_range, .. } = self.inline_composer_context.clone() else {
            return;
        };
        let raw = trimming_whitespaces_and_newlines(&command.command);
        if raw.is_empty() {
            return;
        }
        let command_text = if raw.starts_with('/') { raw.to_owned() } else { format!("/{raw}") };
        let (text, selection) =
            ChatComposerInlineContext::replacing(query_range, &self.state.input_text, &format!("{command_text} "));
        self.inline_invocation_exclusion_tag = None;
        self.set_composer_text(text, selection, cx);
    }

    pub(in super::super) fn select_inline_group_command(&mut self, item: &ChatGroupBotCommandModel, _window: &mut Window, cx: &mut Context<Self>) {
        let ChatComposerInlineContext::GroupCommands { token_range, .. } = self.inline_composer_context.clone() else {
            return;
        };
        let chat_id = self.resolved_chat_id(cx);
        let topic_id = self.inline_composer_topic_id(cx);
        let all = BotServicesStore::chat_commands(&chat_id, topic_id.as_deref(), &self.inline_composer_language_code(), cx)
            .map(|groups| ChatGroupBotCommands::flatten(&groups))
            .unwrap_or_default();
        let (text, selection) = ChatComposerInlineContext::replacing(
            token_range,
            &self.state.input_text,
            &item.insertion(ChatGroupBotCommands::bot_count(&all)),
        );
        self.set_composer_text(text, selection, cx);
    }

    pub(in super::super) fn send_inline_result(&mut self, result: &BotInlineResultModel, window: &mut Window, cx: &mut Context<Self>) {
        let fallback_query_id = self.inline_composer_results(cx).query_id;
        let query_id = result.effective_query_id(&fallback_query_id);
        if query_id.is_empty() {
            return;
        }
        let chat_id = self.resolved_chat_id(cx);
        let topic_id = self.resolved_topic_id(cx);
        let post_topic_id = self.post_topic_id(cx);
        let thread_root_id = self.resolved_thread_root_id().map(str::to_owned);
        let reply_to_id = self.state.reply_target.as_ref().map(|target| target.message_id.clone());
        let on_result: BotInlineSendResult = Rc::new(|success: bool, _: Option<&anyhow::Error>, cx: &mut App| {
            if !success {
                send_failed_notice("chat_inline_send_failed", None, cx);
            }
        });
        let started = bot_interactions(cx, |store, cx| {
            store.send_inline_result(
                &chat_id,
                topic_id.as_deref(),
                post_topic_id.as_deref(),
                thread_root_id.as_deref(),
                reply_to_id.as_deref(),
                &query_id,
                result.clone(),
                Some(on_result),
                cx,
            )
        });
        if !started {
            return;
        }

        self.state.input_text = String::new();
        self.state.reply_target = None;
        self.composer_input_height = ChatBottomBarMetrics::CIRCLE;
        self.composer_selection = NSRange::new(0, 0);
        self.composer_requested_selection = Some(self.composer_selection);
        self.composer_selection_token = self.composer_selection_token.wrapping_add(1);
        self.inline_user_mentions = Vec::new();
        self.draft_custom_emojis = Vec::new();
        message_interactions(cx, |store, cx| store.set_typing(&chat_id, false, ChatAction::Typing, cx));
        self.inline_invocation_exclusion_tag = None;
        self.dismiss_inline_composer(false, window, cx);
        self.follow_outgoing_message(window, cx);
    }

    pub(in super::super) fn retry_inline_composer(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let scope = self.inline_composer_scope(cx);
        bot_interactions(cx, |store, cx| store.cancel_inline_composer(&scope, cx));
        let context = self.inline_composer_context.clone();
        self.request_inline_composer_content(&context, false, cx);
    }

    pub(in super::super) fn load_more_inline_candidates(&mut self, cx: &mut Context<Self>) {
        let ChatComposerInlineContext::Candidates { query, .. } = self.inline_composer_context.clone() else {
            return;
        };
        let chat_id = self.resolved_chat_id(cx);
        let topic_id = self.inline_composer_topic_id(cx);
        let scope = self.inline_composer_scope(cx);
        bot_interactions(cx, |store, cx| store.load_more_mention_candidates(&chat_id, topic_id.as_deref(), &query, &scope, cx));
    }

    pub(in super::super) fn load_more_inline_results(&mut self, cx: &mut Context<Self>) {
        let ChatComposerInlineContext::Bot { bot_tag, query, .. } = self.inline_composer_context.clone() else {
            return;
        };
        let chat_id = self.resolved_chat_id(cx);
        let topic_id = self.inline_composer_topic_id(cx);
        let scope = self.inline_composer_scope(cx);
        bot_interactions(cx, |store, cx| {
            store.load_more_inline_results(&bot_tag, &chat_id, topic_id.as_deref(), &query, &scope, cx);
        });
    }

    pub(in super::super) fn dismiss_inline_composer(&mut self, suppress_current_invocation: bool, _window: &mut Window, cx: &mut Context<Self>) {
        if suppress_current_invocation && let ChatComposerInlineContext::Bot { bot_tag, .. } = &self.inline_composer_context {
            self.inline_invocation_exclusion_tag = Some(bot_tag.clone());
        }
        let scope = self.inline_composer_scope(cx);
        bot_interactions(cx, |store, cx| store.cancel_inline_composer(&scope, cx));
        ChatComposerInsetAnimation::set_last_duration(ChatBottomBarMetrics::COMPOSER_ANIM_DURATION);
        self.inline_composer_context = ChatComposerInlineContext::Inactive;
        self.inline_composer_expanded = false;
        self.inline_composer_interactive_height = None;
        cx.notify();
    }
}
