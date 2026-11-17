use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use gpui::prelude::*;
use gpui::{AnyElement, App, AppContext, Context, Empty, Window, div};
use rise_core::foundation::foundation_date::TimeInterval;
use rise_i18n::{Locale, tr};
use rise_theme::AppTheme;
use rise_ui::swiftui::prelude::*;
use crate::core::rise_engine_bridge::rise_engine_app_runtime::RiseColdOpenTrace;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiWsSliceStatus;
use crate::modules::chat::components::chat::async_render::chat_performance_trace::ChatPerformanceTrace;
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_view::RiseListRemovalDiag;
use crate::modules::chat::components::chat::bubble::message_link_preview_view::MessageVoiceLinkGlimpse;
use crate::modules::chat::components::chat::media_viewer::chat_media_gallery_builder::ChatViewLimitedOpenings;
use crate::modules::chat::components::chat::composer::chat_composer_input_surface::ChatComposerInputSurface;
use crate::modules::chat::components::chat::composer::chat_inline_composer_panel::ChatInlineComposerPresentationContract;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::message::message_actions::message_types::{GetMessageMessage, MessageContentType};
use crate::modules::chat::stores::message::message_services::message_presentation_window_projector::MessagePresentationWindowProjector;
use crate::modules::chat::stores::message::message_services::message_services::{MessageScopePresentationSnapshot, MessageServicesStore};
use std::hash::{DefaultHasher, Hash, Hasher};
use rise_ui::animated_media_playback_scope::{AnimatedMediaPausesUnderMediaViewerKey, AnimatedMediaPlaybackActiveKey};
use crate::modules::chat::components::chat::media_viewer::location_fullscreen::location_presentation;
use rise_ui::loader_ui::{LoaderUi, LoaderUiSize};
use rise_ui::swiftui::environment::env_scope;
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::SaiDataRender;
use crate::core::lib::sai::modules::sai_ws::sai_cache_revision_tracker::SaiCacheRevisionTracker;
use crate::modules::chat::components::chat::async_render::rise_message_list_view::{RiseMessageFeedSourceVersionPolicy, RiseMessageViewportState};
use crate::modules::chat::components::chat::chat_message_feed::ChatMessageFeed;
use crate::modules::chat::components::chat::message_row::ChatStickyBottomLimitKey;
use crate::modules::chat::engine::messages::rise_message_scope::RiseMessagePageDirection;
use crate::modules::chat::engine::messages::rise_message_unread_marks::RiseUnreadMarkViewport;
use crate::modules::chat::shared::chat_search_highlight::ChatMessageSearchHighlight;
use crate::modules::chat::stores::chats::chat_actions::chat_types::User;
use crate::modules::chat::stores::message::message_actions::message_types::MessageHistoryDirection;
use crate::modules::chat::stores::message::message_services::message_presentation_window_projector::MessagePresentationWindowState;
use super::super::chat_screen::*;
use super::super::chat_screen_support::*;

impl ChatScreen {
    pub(in super::super) fn visible_messages(
        &self,
        bounded_presentation: Option<&MessageScopePresentationSnapshot>,
        cx: &mut App,
    ) -> Vec<GetMessageMessage> {
        let scope = self.message_scope(cx);
        let bounded = match bounded_presentation {
            Some(snapshot) => snapshot.slice.messages.clone(),
            None => message_services(cx, |store, cx| store.message_presentation_slice(&scope, cx)).messages,
        };
        let live = if bounded.is_empty() {
            let canonical = message_services(cx, |store, cx| store.messages_in_memory(&scope, cx));
            if ChatMessagePresentationRecoveryPolicy::should_use_canonical_fallback(
                bounded.len() as i64,
                canonical.len() as i64,
            ) {
                let start = canonical.len().saturating_sub(MessagePresentationWindowProjector::limit() as usize);
                let live = canonical[start..].to_vec();
                let trace = ChatPerformanceTrace::shared();
                if trace.is_recording() {
                    trace.record(
                        "presentation.empty_slice_recovered",
                        HashMap::from([
                            ("scope".to_owned(), scope.clone()),
                            ("canonical".to_owned(), canonical.len().to_string()),
                            ("recovered".to_owned(), live.len().to_string()),
                        ]),
                    );
                }
                live
            } else {
                bounded
            }
        } else {
            bounded
        };
        match self.resolved_thread_root_id() {
            Some(root_id) => Self::thread_presentation(live, root_id, &self.resolved_chat_id(cx)),
            None => live,
        }
    }

    pub(in super::super) fn thread_presentation(messages: Vec<GetMessageMessage>, root_id: &str, chat_id: &str) -> Vec<GetMessageMessage> {
        let mut out = Vec::with_capacity(messages.len() + 1);
        let mut root_created_at: TimeInterval = 0.0;
        for mut message in messages {
            if message.matches_message_id(root_id) {
                root_created_at = message.created_at;
            } else if message.reply_to.as_ref().is_some_and(|reply| reply.message_id == root_id) {
                message.reply_to = None;
            }
            out.push(message);
        }
        if root_created_at <= 0.0 {
            return out;
        }
        let mut pill = GetMessageMessage::new(format!("thread-start-{root_id}"));
        pill.chat_id = chat_id.to_owned();
        pill.content_type = MessageContentType::System;
        pill.text = tr("chat_thread_start");
        pill.created_at = root_created_at + 0.001;
        if let Some(root_index) = out.iter().position(|message| message.matches_message_id(root_id)) {
            out.insert(root_index + 1, pill);
        }
        out
    }

    pub(in super::super) fn message_load_task_id(&self, cx: &mut App) -> ChatMessageLoadTaskID {
        ChatMessageLoadTaskID {
            scope: self.message_scope(cx),
            scroll_position_key: self.scroll_position_key(cx),
            navigation_open_epoch: self.navigation_open_epoch,
            viewer_removed_from_community: self.viewer_removed_from_community(cx),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn watch_location_cover(&mut self, cx: &mut Context<Self>) {
        let Some(presenter) = location_presentation(cx) else {
            return;
        };
        if self.location_cover_watch.as_ref().is_some_and(|(id, _)| *id == presenter.entity_id()) {
            return;
        }
        let subscription = cx.observe(&presenter, |screen, presenter, cx| {
            let covers = presenter.read(cx).is_presented();
            if covers != screen.location_covers_feed {
                screen.location_covers_feed = covers;
                cx.notify();
            }
        });
        self.location_cover_watch = Some((presenter.entity_id(), subscription));
    }

    pub(in super::super) fn message_feed_layer(
        &mut self,
        chat: &ChatInfo,
        messages: &[GetMessageMessage],
        feed_rows: &[Arc<GetMessageMessage>],
        bounded_presentation: Option<&MessageScopePresentationSnapshot>,
        reply_keyboard_panel_height: f32,
        inline_panel_height: f32,
        theme: &AppTheme,
        safe_bottom: f32,
        peer_blocked: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let chrome_panel_height = inline_panel_height.max(0.0) + self.unread_mark_buttons_lift(cx);
        let feed_inset = ChatInlineComposerPresentationContract::feed_bottom_inset(
            self.composer_total_height(safe_bottom, reply_keyboard_panel_height, peer_blocked, cx) + chrome_panel_height,
            ChatInlineComposerPresentationContract::FEED_BOTTOM_SPACING,
        );
        let resting_safe_bottom = safe_bottom.max(Self::chat_window_safe_bottom_inset());
        let keyboard_resting_feed_inset = ChatInlineComposerPresentationContract::feed_bottom_inset(
            self.composer_total_height(resting_safe_bottom, reply_keyboard_panel_height, peer_blocked, cx) + chrome_panel_height,
            ChatInlineComposerPresentationContract::FEED_BOTTOM_SPACING,
        );
        let keyboard_active_feed_inset = ChatInlineComposerPresentationContract::feed_bottom_inset(
            self.composer_total_height(0.0, reply_keyboard_panel_height, peer_blocked, cx) + chrome_panel_height,
            ChatInlineComposerPresentationContract::FEED_BOTTOM_SPACING,
        );
        let is_context_preview = self.is_context_preview(cx);
        let is_message_request = self.is_message_request(cx);
        let tracks_system_keyboard =
            !is_context_preview && self.composer_input_surface == ChatComposerInputSurface::SystemKeyboard && !self.picker_visible;

        let message_scope = self.message_scope(cx);
        let owned_slice = match bounded_presentation {
            Some(_) => None,
            None => Some(message_services(cx, |store, cx| store.message_presentation_slice(&message_scope, cx))),
        };
        let Some(presentation_slice) = bounded_presentation.map(|snapshot| &snapshot.slice).or(owned_slice.as_ref()) else {
            return Empty.into_any_element();
        };
        let has_cached_older = presentation_slice.has_cached_before();
        let has_cached_newer = presentation_slice.has_cached_after();
        let can_load_older = has_cached_older
            || match bounded_presentation {
                Some(snapshot) => snapshot.has_more_before,
                None => message_services(cx, |store, cx| store.has_more_messages(&message_scope, MessageHistoryDirection::Older, cx)),
            };
        let can_load_newer = has_cached_newer
            || match bounded_presentation {
                Some(snapshot) => snapshot.has_more_after,
                None => message_services(cx, |store, cx| store.has_more_messages(&message_scope, MessageHistoryDirection::Newer, cx)),
            };
        let canonical_count = match bounded_presentation {
            Some(snapshot) => snapshot.slice.messages.len() as i64,
            None => message_services(cx, |store, cx| store.in_memory_message_count(&message_scope, cx)),
        };
        let request_status = match bounded_presentation {
            Some(snapshot) => snapshot.request_status.clone(),
            None => message_services(cx, |store, cx| store.initial_messages_status(&message_scope, cx)),
        };
        let is_pending_chat = self.is_pending_chat(cx);
        let request_in_flight = MessageServicesStore::shared(cx).read(cx).is_initial_messages_request_in_flight(&message_scope)
            || (!is_pending_chat && self.initial_network_load_scope.as_deref() != Some(message_scope.as_str()));
        let unread_separator_message_id = self.unread_divider_message_id(messages, cx);
        let awaits_unread_divider = if self.viewer_removed_from_community(cx) {
            false
        } else if self.unread_divider.scope.as_deref() == Some(message_scope.as_str()) {
            self.unread_divider.count > 0 && unread_separator_message_id.is_none()
        } else {
            !self.is_context_preview(cx) && self.active_unread_count(cx) > 0
        };
        let opening_request_pending = self.opening_load.as_ref().is_some_and(|opening| opening.scope == message_scope)
            || (!is_pending_chat && self.initial_network_load_scope.as_deref() != Some(message_scope.as_str()));
        let awaits_first_sync = match self.opening_load.as_ref().filter(|opening| opening.scope == message_scope) {
            Some(opening) => opening.awaits_first_sync,
            None => {
                opening_request_pending
                    && !is_pending_chat
                    && !is_context_preview
                    && ChatOpeningSyncedScopes::awaits_first_sync(&message_scope, cx)
            }
        };
        let opening_resolved = ChatMessagePresentationRecoveryPolicy::initial_data_resolved(
            is_pending_chat,
            messages.len() as i64,
            canonical_count,
            request_in_flight,
            request_status.clone(),
            false,
            awaits_unread_divider || awaits_first_sync,
            opening_request_pending,
        );
        let initial_data_resolved = opening_resolved
            || (self.context_preview.as_ref().is_some_and(|session| session.read(cx).presents_adopted_feed())
                && request_status != SaiWsSliceStatus::Rejected
                && !messages.is_empty());
        let feed_count = messages.len();
        let watcher = cx.weak_entity();
        Self::watch(&watcher, "chat-initial-data-resolved", initial_data_resolved, false, window, cx, move |screen, _, resolved, _, cx| {
            let Some(preview) = screen.context_preview.as_ref() else {
                return;
            };
            let adopted = preview.read(cx).presents_adopted_feed();
            let resolved = *resolved;
            RiseListRemovalDiag::log(
                move || format!("[d8f4] feed resolved={resolved} opening={opening_resolved} adopted={adopted} n={feed_count}"),
                false,
                cx,
            );
        });
        let x357_state = format!(
            "chat={message_scope} n={feed_count} canonical={canonical_count} resolved={initial_data_resolved} opening={opening_resolved} pending_open={opening_request_pending} awaits_divider={awaits_unread_divider} in_flight={request_in_flight} status={request_status:?}"
        );
        Self::watch(&watcher, "x357-feed-state", x357_state, true, window, cx, |_, _, state, _, _| {
            tracing::warn!(target: "riseonly::X357", "feed {state}");
        });
        let feed_status = if initial_data_resolved {
            SaiWsSliceStatus::Fulfilled
        } else if request_status == SaiWsSliceStatus::Rejected {
            SaiWsSliceStatus::Rejected
        } else {
            SaiWsSliceStatus::Pending
        };
        let cache_revision = match bounded_presentation {
            Some(snapshot) => snapshot.revision as i64,
            None => {
                let key = MessageServicesStore::messages_cache_key(&message_scope);
                SaiCacheRevisionTracker::observed_revision(&key, cx)
            }
        };
        let feed_topic_titles = self.topic_titles_map(cx);
        let search_highlight = self.current_message_search_highlight(cx);
        let group_by_topic = self.group_feed_by_topic(cx);
        let source_version = self.message_feed_source_version(
            cache_revision,
            Some(&presentation_slice.state),
            (messages.first().map(|message| message.id.as_str()), messages.last().map(|message| message.id.as_str())),
            chat,
            &feed_topic_titles,
            search_highlight.as_ref(),
            unread_separator_message_id.as_deref(),
            cx,
        );

        let mut feed = ChatMessageFeed::new(
            "chat-message-feed",
            feed_rows.to_vec(),
            Arc::new(chat.clone()),
            message_scope.clone(),
            feed_inset,
            source_version,
        );
        let chat_id = self.resolved_chat_id(cx);
        let topic_id = self.resolved_topic_id(cx);
        let thread_root_id = self.resolved_thread_root_id().map(str::to_owned);
        feed.gallery_paginates = thread_root_id.is_none();
        let read_topic_id = self.read_topic_id(cx);
        let this = cx.weak_entity();

        feed.scroll_position_key = Some(self.scroll_position_key(cx));
        feed.sticky_bottom_inset = Some(feed_inset - 10.0);
        feed.keyboard_resting_bottom_inset = Some(keyboard_resting_feed_inset);
        feed.keyboard_resting_sticky_bottom_inset = Some(keyboard_resting_feed_inset - 10.0);
        feed.keyboard_active_bottom_inset = Some(keyboard_active_feed_inset);
        feed.keyboard_active_sticky_bottom_inset = Some(keyboard_active_feed_inset - 10.0);
        feed.tracks_system_keyboard = tracks_system_keyboard;
        feed.settles_composer_inset_immediately = is_context_preview;
        feed.content_stamps_by_local_id =
            bounded_presentation.map(|snapshot| snapshot.content_stamps_by_local_id.clone()).unwrap_or_default();
        feed.presentation_epoch = self.navigation_open_epoch;
        feed.interactions_enabled = !is_message_request;
        feed.scroll_request = self.message_list_scroll_request.clone();
        feed.allows_message_selection = !is_message_request;
        feed.message_selection_active = self.state.is_message_selection_active;
        feed.selected_message_ids = self.selected_message_ids(cx);
        feed.group_by_topic = group_by_topic;
        feed.topic_titles = feed_topic_titles;
        feed.can_load_older = can_load_older;
        feed.can_load_newer = can_load_newer;
        feed.has_cached_older = has_cached_older;
        feed.has_cached_newer = has_cached_newer;
        feed.initial_data_resolved = initial_data_resolved;
        feed.search_highlight = search_highlight;
        feed.unread_separator_message_id = unread_separator_message_id;

        let screen = this.clone();
        feed.on_toggle_message_selection = Some(Rc::new(move |message: &GetMessageMessage, window: &mut Window, cx: &mut App| {
            let _ = screen.update(cx, |screen, cx| {
                if !screen.is_context_preview(cx) {
                    screen.toggle_message_selection(message, window, cx);
                }
            });
        }));
        let screen = this.clone();
        feed.on_reaction = Some(Rc::new(move |message_id: &str, emoji: &str, window: &mut Window, cx: &mut App| {
            let _ = screen.update(cx, |screen, cx| {
                if !screen.is_context_preview(cx) {
                    screen.handle_message_reaction(message_id, emoji, window, cx);
                }
            });
        }));
        let screen = this.clone();
        feed.on_action = Some(Rc::new(move |message_id: &str, action_id: &str, window: &mut Window, cx: &mut App| {
            let _ = screen.update(cx, |screen, cx| {
                if !screen.is_context_preview(cx) {
                    screen.handle_message_action(message_id, action_id, window, cx);
                }
            });
        }));
        let screen = this.clone();
        feed.on_open_sender_profile = Some(Rc::new(move |participant: &User, window: &mut Window, cx: &mut App| {
            let _ = screen.update(cx, |screen, cx| {
                if !screen.is_context_preview(cx) {
                    screen.open_sender_chat_profile(participant, window, cx);
                }
            });
        }));
        let screen = this.clone();
        feed.on_select_topic = Some(Rc::new(move |topic_id: Option<&str>, window: &mut Window, cx: &mut App| {
            let _ = screen.update(cx, |screen, cx| {
                if screen.is_context_preview(cx) {
                    return;
                }
                let tab = match topic_id.filter(|id| !id.is_empty()) {
                    Some(id) => ChatTopicTab::Topic(id.to_owned()),
                    None => ChatTopicTab::General,
                };
                screen.select_topic_tab(tab, window, cx);
            });
        }));
        let screen = this.clone();
        feed.on_select_date = Some(Rc::new(move |day_start_ms: i64, is_pinned: bool, window: &mut Window, cx: &mut App| {
            let _ = screen.update(cx, |screen, cx| {
                if !screen.is_context_preview(cx) {
                    screen.handle_date_separator_tap(day_start_ms, is_pinned, window, cx);
                }
            });
        }));
        let page_ids = Rc::new((chat_id.clone(), topic_id, thread_root_id));
        let ids = page_ids.clone();
        feed.on_load_older = Some(Rc::new(move |visible_anchor_id: Option<&str>, _window: &mut Window, cx: &mut App| {
            message_interactions(cx, |store, cx| {
                store.load_more_messages(&ids.0, ids.1.as_deref(), ids.2.as_deref(), true, visible_anchor_id, cx);
            });
        }));
        let ids = page_ids.clone();
        feed.on_load_newer = Some(Rc::new(move |visible_anchor_id: Option<&str>, _window: &mut Window, cx: &mut App| {
            message_interactions(cx, |store, cx| {
                store.load_newer_messages(&ids.0, ids.1.as_deref(), ids.2.as_deref(), true, visible_anchor_id, cx);
            });
        }));
        let ids = page_ids;
        feed.on_prefetch_page =
            Some(Rc::new(move |direction: Option<RiseMessagePageDirection>, _window: &mut Window, cx: &mut App| {
                message_interactions(cx, |store, cx| {
                    store.prefetch_message_page(&ids.0, ids.1.as_deref(), ids.2.as_deref(), direction, cx);
                });
            }));
        let screen = this.clone();
        feed.on_initial_frame_ready = Some(Rc::new(move |_window: &mut Window, cx: &mut App| {
            let _ = screen.update(cx, |screen, cx| screen.handle_feed_initial_frame_ready(cx));
        }));
        let scrolling_scope = message_scope.clone();
        feed.on_scroll_activity_changed = Some(Rc::new(move |is_scrolling: bool, _window: &mut Window, cx: &mut App| {
            message_interactions(cx, |store, cx| store.set_message_list_scrolling(is_scrolling, &scrolling_scope, cx));
        }));
        let screen = this.clone();
        feed.on_viewport_state_changed = Some(Rc::new(move |viewport_state: &RiseMessageViewportState, window: &mut Window, cx: &mut App| {
            let (is_at_latest, shows_jump_to_latest) = (viewport_state.is_at_latest, viewport_state.shows_jump_to_latest);
            let _ = screen.update(cx, |screen, cx| {
                screen.handle_message_viewport_state(is_at_latest, shows_jump_to_latest, window, cx);
            });
        }));
        let marks_chat_id = chat_id;
        let partial_read_screen = this.clone();
        feed.on_visible_mark_candidates_changed = Some(Rc::new(move |viewport: RiseUnreadMarkViewport, _window: &mut Window, cx: &mut App| {
            if is_context_preview
                || partial_read_screen.update(cx, |screen, cx| screen.is_hidden_under_sibling_topic(cx)).unwrap_or(false)
            {
                return;
            }
            let (chat_id, topic_id) = (marks_chat_id.clone(), read_topic_id.clone());
            let current = viewport.clone();
            message_interactions(cx, |store, cx| {
                store.unread_mark_candidates_became_visible(&chat_id, topic_id.as_deref(), viewport, cx);
            });
            let screen = partial_read_screen.clone();
            cx.spawn(async move |cx| {
                let visible_message_ids: Vec<String> =
                    current.current_candidates().await.into_iter().map(|candidate| candidate.message_id).collect();
                let _ = screen.update(cx, |screen, cx| screen.report_partial_read_progress(visible_message_ids, cx));
            })
            .detach();
        }));

        let sticky_limit = self.content_height - feed_inset;
        let feed_element = env_scope::<ChatStickyBottomLimitKey>(sticky_limit).child(feed);

        let loading_bg = theme.bg._100.opacity(0.35);
        let loader_color = theme.primary._100;
        let error_text = if self.shows_thread_members_only_hint(cx) {
            tr("chat_thread_members_only")
        } else {
            message_services(cx, |store, cx| store.initial_messages_error(&message_scope, cx))
                .unwrap_or_else(|| tr("error_fetching_data"))
        };
        let error_theme = theme.clone();
        let on_retry: Option<Rc<dyn Fn(&mut Window, &mut App)>> = (!is_context_preview).then(|| {
            let retry: Rc<dyn Fn(&mut Window, &mut App)> = Rc::new(Self::act(this, |screen, _, cx| screen.retry_initial_messages(cx)));
            retry
        });
        let rendered = SaiDataRender::new(feed_status, move |_, _| feed_element)
            .is_empty(false)
            .need_pending(true)
            .message_height_percent(50.0)
            .sai_data_render_loading(move |_, _| {
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(loading_bg)
                    .child(LoaderUi::new().size(LoaderUiSize::Large).color(Some(loader_color)))
            })
            .sai_data_render_error(move |_, _| ChatMessagesLoadErrorState {
                text: error_text.clone(),
                theme: error_theme.clone(),
                on_retry: on_retry.clone(),
            });
        let frame_ready = match self.navigation_open_epoch {
            Some(epoch) => self.first_frame_ready_epoch == Some(epoch),
            None => self.first_frame_ready_scope.as_deref() == Some(message_scope.as_str()),
        };
        let awaits_first_frame = initial_data_resolved && feed_count > 0 && !frame_ready && !is_context_preview;
        let body = if awaits_first_frame {
            div()
                .relative()
                .size_full()
                .child(rendered)
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(loading_bg)
                        .child(LoaderUi::new().size(LoaderUiSize::Large).color(Some(loader_color))),
                )
                .into_any_element()
        } else {
            rendered.into_any_element()
        };
        self.watch_location_cover(cx);
        env_scope::<AnimatedMediaPlaybackActiveKey>(
            self.is_message_media_playback_active && !self.show_gif_search && !self.location_covers_feed,
        )
            .child(env_scope::<AnimatedMediaPausesUnderMediaViewerKey>(true).child(body))
            .into_any_element()
    }

    pub(in super::super) fn handle_feed_initial_frame_ready(&mut self, cx: &mut Context<Self>) {
        RiseColdOpenTrace::mark(|| "chat initial frame ready".to_owned());
        if let Some(context_preview) = self.context_preview.clone() {
            context_preview.update(cx, |session, cx| session.initial_frame_ready(cx));
        }
        let scope = self.message_scope(cx);
        if let Some(epoch) = self.navigation_open_epoch {
            if self.first_frame_ready_epoch == Some(epoch) {
                return;
            }
            self.first_frame_ready_epoch = Some(epoch);
        } else if self.first_frame_ready_scope.as_deref() == Some(scope.as_str()) {
            return;
        }
        self.first_frame_ready_scope = Some(scope);
        cx.notify();
        let route = self.first_frame_navigation_route();
        let epoch = self.navigation_open_epoch;
        if let Some(nav_store) = self.nav_store() {
            nav_store.update(cx, |store, cx| store.complete_chat_first_frame_transition(&route, epoch, cx));
        }
    }

    pub(in super::super) fn retry_initial_messages(&mut self, cx: &mut Context<Self>) {
        if self.resolved_chat_id(cx).is_empty() || self.is_pending_chat(cx) {
            return;
        }
        self.load_initial_messages(cx);
    }

    pub(in super::super) fn chat_window_safe_bottom_inset() -> f32 {
        0.0
    }

    #[allow(clippy::too_many_arguments)]
    pub(in super::super) fn message_feed_source_version(
        &self,
        cache_revision: i64,
        presentation_state: Option<&MessagePresentationWindowState>,
        window_edge_ids: (Option<&str>, Option<&str>),
        chat: &ChatInfo,
        topic_titles: &HashMap<String, String>,
        search_highlight: Option<&ChatMessageSearchHighlight>,
        unread_separator_message_id: Option<&str>,
        cx: &mut App,
    ) -> i64 {
        let mut hasher = DefaultHasher::new();
        unread_separator_message_id.hash(&mut hasher);
        search_highlight.hash(&mut hasher);
        let mut view_limited_closed: Vec<String> = ChatViewLimitedOpenings::shared(cx).read(cx).closed().iter().cloned().collect();
        view_limited_closed.sort();
        view_limited_closed.hash(&mut hasher);
        MessageVoiceLinkGlimpse::shared(cx).read(cx).revision().hash(&mut hasher);
        RiseMessageFeedSourceVersionPolicy::data_version(cache_revision, presentation_state).hash(&mut hasher);
        window_edge_ids.0.hash(&mut hasher);
        window_edge_ids.1.hash(&mut hasher);
        chat.id.hash(&mut hasher);
        format!("{:?}", chat.r#type).hash(&mut hasher);
        chat.allowed_reactions.hash(&mut hasher);
        chat.role.hash(&mut hasher);
        chat.forward_in_chat_enabled.hash(&mut hasher);
        chat.can_pin_messages.hash(&mut hasher);
        chat.can_manage_chat.hash(&mut hasher);
        Locale::app_locale().identifier().hash(&mut hasher);
        self.resolved_thread_root_id().hash(&mut hasher);
        let group_by_topic = self.group_feed_by_topic(cx);
        group_by_topic.hash(&mut hasher);
        if self.is_topics_mode(cx) {
            match &self.state.selected_topic_tab {
                ChatTopicTab::All => "all".hash(&mut hasher),
                ChatTopicTab::General => "general".hash(&mut hasher),
                ChatTopicTab::Topic(id) => {
                    "topic".hash(&mut hasher);
                    id.hash(&mut hasher);
                }
            }
        }
        if group_by_topic {
            let mut keys: Vec<&String> = topic_titles.keys().collect();
            keys.sort();
            for key in keys {
                key.hash(&mut hasher);
                topic_titles.get(key).hash(&mut hasher);
            }
        }
        hasher.finish() as i64
    }
}
