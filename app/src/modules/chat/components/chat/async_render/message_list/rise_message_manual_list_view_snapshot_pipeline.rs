use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use gpui::{App, AppContext as _, AsyncApp, Context, Hsla, Window};
use rise_core::foundation::foundation_calendar::{Calendar, CalendarIdentifier, TimeZone};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::language_store::Locale;
use crate::core::rise_engine_bridge::rise_engine_app_runtime::RiseColdOpenTrace;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::modules::chat::components::chat::async_render::chat_message_render_snapshot::{ChatMessageRenderEntryKind, ChatMessageRenderPreparationSeed, ChatMessageRenderRequestKey, ChatMessageRenderRequestPolicy, ChatMessageRenderRequestPolicyAction, ChatMessageRenderSnapshot};
use crate::modules::chat::components::chat::async_render::chat_performance_trace::ChatPerformanceTrace;
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_types::{RiseManualListScrollAnimationPolicy, RiseManualListSelectionItem, RiseManualListSelectionLayout, RiseManualListSelectionPresentation, RiseManualListSelectionPresentationOnToggle, RiseManualListTransaction, RiseManualListTransactionAnchor, RiseManualListTransactionJumpAnimation, RiseManualListTransactionUpdateAnimation};
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_view::RiseManualListView;
use crate::modules::chat::components::chat::message_list::MessageList;
use crate::modules::chat::components::chat::async_render::async_text_bubble_predisplay::AsyncTextBubblePredisplayCache;
use crate::core::ui::clever_image_disk_cache::CleverImageDiskPrefetch;
use crate::core::ui::lottie_sticker::lottie_sticker_prefetch::LottieStickerPrefetch;
use crate::modules::chat::components::chat::scroll::chat_scroll_position_store::ChatScrollPositionStore;
use crate::modules::chat::engine::core::rise_conversation_presentation::RiseMessagePresentationContentStamp;
use crate::modules::chat::shared::chat_search_highlight::ChatMessageSearchHighlight;
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::chat::shared::message_permissions::MessagePermissions;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::message::message_actions::message_types::GetMessageMessage;
use crate::modules::chat::stores::message::message_services::message_pagination_trace::MessagePaginationTrace;
use crate::modules::chat::stores::message::message_services::message_presentation_window_projector::MessagePresentationWindowProjector;
use super::super::rise_message_list_view::*;

impl RiseMessageManualListView {
    #[allow(clippy::too_many_arguments)]
    pub fn request_render_snapshot(
        &mut self,
        messages: Vec<Arc<GetMessageMessage>>,
        chat_id: &str,
        source_version: i64,
        content_stamps_by_local_id: HashMap<String, RiseMessagePresentationContentStamp>,
        group_by_topic: bool,
        topic_titles: HashMap<String, String>,
        locale: Locale,
        calendar: Calendar,
        time_zone: TimeZone,
        presentation_epoch: Option<Uuid>,
        saved_messages_chat: bool,
        search_highlight: Option<ChatMessageSearchHighlight>,
        unread_separator_message_id: Option<String>,
        view_limited_opened: HashSet<String>,
        view_limited_closed: HashSet<String>,
        completion: RiseMessageManualListViewRenderCompletion,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let key = ChatMessageRenderRequestKey {
            account_user_id: account_user_id.clone(),
            account_epoch: account_epoch.clone(),
            chat_id: chat_id.to_owned(),
            source_version,
            group_by_topic,
            topic_titles: if group_by_topic { topic_titles.clone() } else { HashMap::new() },
            locale_identifier: locale.identifier().to_owned(),
            calendar_identifier: match calendar.identifier {
                CalendarIdentifier::Gregorian => "gregorian".to_owned(),
            },
            time_zone_identifier: time_zone.identifier().to_owned(),
            saved_messages_chat,
            view_limited_opened: view_limited_opened.clone(),
            view_limited_closed: view_limited_closed.clone(),
        };

        let starts_new_presentation = RiseMessageFirstFramePresentationPolicy::starts_new_presentation(
            self.presentation_epoch,
            presentation_epoch,
            self.presentation_chat_id
                .as_deref()
                .is_some_and(|presentation_chat_id| presentation_chat_id != chat_id),
        );
        let changes_account = self.media_prefetch_account_user_id != account_user_id
            || self.media_prefetch_account_epoch != account_epoch;
        let incoming_presentation_commit = RiseMessagePendingPresentationCommit {
            account_user_id: account_user_id.clone(),
            account_epoch: account_epoch.clone(),
            chat_id: chat_id.to_owned(),
            presentation_epoch,
            source_version,
        };
        let should_reset_presentation = (starts_new_presentation || changes_account)
            && self.pending_presentation_commit.as_ref().is_none_or(|pending| {
                !pending.belongs_to_presentation(
                    account_user_id.as_deref(),
                    account_epoch.as_deref(),
                    chat_id,
                    presentation_epoch,
                )
            });
        if RiseMessagePendingPresentationCommitPolicy::should_track_incoming(
            self.pending_presentation_commit.as_ref(),
            &incoming_presentation_commit,
            starts_new_presentation,
            changes_account,
        ) {
            self.pending_presentation_commit = Some(incoming_presentation_commit);
        }
        if should_reset_presentation {
            self.invalidate_history_edge_for_pending_presentation();
            self.list_view
                .update(cx, |list_view, _| list_view.cancel_visible_viewport_preparation());
            self.list_alpha = 0.0;
            cx.notify();
        }

        let request_action = ChatMessageRenderRequestPolicy::action(
            self.snapshot_cache.as_ref().map(|cached| &cached.key),
            self.pending_render_key.as_ref(),
            self.render_preparation_task.is_some(),
            &key,
        );
        if request_action == ChatMessageRenderRequestPolicyAction::UseCached
            && let Some(cached) = self.snapshot_cache.as_ref().map(|cached| cached.snapshot.clone())
        {
            self.render_preparation_generation = self.render_preparation_generation.wrapping_add(1);
            self.cancel_render_preparation_task();
            self.pending_render_key = None;
            self.pending_render_completion = None;
            completion(cached, self, window, cx);
            return;
        }

        if request_action == ChatMessageRenderRequestPolicyAction::CoalesceInFlight {
            self.pending_render_completion = Some(completion);
            return;
        }

        self.render_preparation_generation = self.render_preparation_generation.wrapping_add(1);
        let generation = self.render_preparation_generation;
        self.cancel_render_preparation_task();
        self.pending_render_key = Some(key.clone());
        self.pending_render_completion = Some(completion);
        let previous = self
            .snapshot_cache
            .as_ref()
            .filter(|cached| cached.key.chat_id == chat_id)
            .map(|cached| cached.snapshot.clone());
        let message_count = messages.len();
        let seed = ChatMessageRenderPreparationSeed::make(
            messages,
            chat_id,
            source_version,
            content_stamps_by_local_id,
            group_by_topic,
            topic_titles,
            locale,
            &calendar,
            time_zone,
            account_user_id,
            account_epoch,
            saved_messages_chat,
            search_highlight,
            unread_separator_message_id,
            view_limited_opened,
            view_limited_closed,
            cx,
        );
        let traces_performance = ChatPerformanceTrace::shared().is_enabled();
        let trace_token = ChatPerformanceTrace::shared().begin_operation(
            "snapshot.build",
            HashMap::from([
                ("messages".to_owned(), message_count.to_string()),
                ("source".to_owned(), source_version.to_string()),
                ("worker".to_owned(), "actor".to_owned()),
            ]),
        );
        let cancelled = Arc::new(AtomicBool::new(false));
        self.render_preparation_cancelled = Some(cancelled.clone());
        let preparer = self.render_snapshot_preparer.clone();
        self.render_preparation_task = Some(cx.spawn_in(window, async move |this, cx| {
            let worker_cancelled = cancelled.clone();
            let result = cx
                .background_spawn(async move {
                    preparer.prepare(&seed, previous.as_deref(), traces_performance, &|| {
                        worker_cancelled.load(Ordering::Relaxed)
                    })
                })
                .await;
            let delivered = this.update_in(cx, |this, window, cx| {
                let active_account_user_id = SaiAccountCacheScope::active_user_id();
                let active_account_epoch = SaiAccountCacheScope::active_epoch();
                let accepts_result = ChatMessageRenderRequestPolicy::accepts_prepared_result(
                    generation,
                    this.render_preparation_generation,
                    &key,
                    this.pending_render_key.as_ref(),
                    active_account_user_id.as_deref(),
                    active_account_epoch.as_deref(),
                );
                let result = match result {
                    Some(result) if accepts_result && !cancelled.load(Ordering::Relaxed) => result,
                    _ => {
                        if this.render_preparation_generation == generation
                            && this.pending_render_key.as_ref() == Some(&key)
                        {
                            this.render_preparation_task = None;
                            this.render_preparation_cancelled = None;
                            this.pending_render_key = None;
                            this.pending_render_completion = None;
                        }
                        ChatPerformanceTrace::shared().end_operation(
                            trace_token,
                            1.0,
                            HashMap::from([("result".to_owned(), "cancelled".to_owned())]),
                        );
                        return;
                    }
                };

                this.render_preparation_task = None;
                this.render_preparation_cancelled = None;
                this.pending_render_key = None;
                let snapshot = Arc::new(result.snapshot);
                this.snapshot_cache = Some(RiseMessageManualListViewSnapshotCacheValue {
                    key,
                    snapshot: snapshot.clone(),
                });
                if traces_performance {
                    ChatPerformanceTrace::shared().record("snapshot.build.detail", result.metrics.trace_fields());
                }
                ChatPerformanceTrace::shared().end_operation(
                    trace_token,
                    1.0,
                    HashMap::from([
                        ("entries".to_owned(), snapshot.entries.len().to_string()),
                        ("inserted".to_owned(), snapshot.delta.inserted_ids.len().to_string()),
                        ("removed".to_owned(), snapshot.delta.removed_ids.len().to_string()),
                        ("updated".to_owned(), snapshot.delta.updated_ids.len().to_string()),
                        ("result".to_owned(), "prepared".to_owned()),
                    ]),
                );
                if let Some(latest_completion) = this.pending_render_completion.take() {
                    latest_completion(snapshot, this, window, cx);
                }
            });
            drop(delivered);
        }));
    }

    pub(in super::super) fn cancel_render_preparation_task(&mut self) {
        if let Some(cancelled) = self.render_preparation_cancelled.take() {
            cancelled.store(true, Ordering::Relaxed);
        }
        self.render_preparation_task = None;
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_selection_presentation(
        &mut self,
        snapshot: &ChatMessageRenderSnapshot,
        chat: &ChatInfo,
        allows_message_selection: bool,
        message_selection_active: bool,
        selected_message_ids: &HashSet<String>,
        tint_color: Hsla,
        on_toggle_message_selection: Option<RiseMessageListViewToggleSelectionHandler>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !allows_message_selection || !message_selection_active {
            self.list_view.update(cx, |list_view, cx| {
                list_view.apply_selection_presentation(RiseManualListSelectionPresentation::inactive(), window, cx);
            });
            return;
        }

        let mut items_by_item_id: HashMap<String, RiseManualListSelectionItem> =
            HashMap::with_capacity(snapshot.entries.len());
        let mut messages_by_selection_id: HashMap<String, Arc<GetMessageMessage>> =
            HashMap::with_capacity(snapshot.entries.len());

        for entry in &snapshot.entries {
            if entry.kind != ChatMessageRenderEntryKind::Message && entry.kind != ChatMessageRenderEntryKind::System {
                continue;
            }
            let Some(message) = entry.message.as_ref() else {
                continue;
            };
            let is_system_row = entry.kind == ChatMessageRenderEntryKind::System;
            let is_own = !is_system_row && MessageList::is_own_bubble(message, chat, cx);
            let reserves_avatar_space = MessageSizes::reserves_avatar_space(chat, is_own);
            let server_id = message.sai_server_id();
            let canonical_id = trimming_whitespaces_and_newlines(&server_id).to_owned();
            let can_select = MessagePermissions::new(message, chat, is_own).can_select(cx);
            let selection_id = can_select.then_some(canonical_id);
            if let Some(selection_id) = &selection_id {
                messages_by_selection_id.insert(selection_id.clone(), message.clone());
            }
            items_by_item_id.insert(
                entry.id.clone(),
                RiseManualListSelectionItem {
                    item_id: entry.id.clone(),
                    selection_id,
                    indicator_center_x: RiseManualListSelectionLayout::indicator_center_x(),
                    content_translation_x: if is_system_row {
                        RiseManualListSelectionLayout::indicator_lane_width()
                    } else {
                        RiseManualListSelectionLayout::content_translation_x(is_own, reserves_avatar_space)
                    },
                    accessibility_label: if is_system_row {
                        entry.label.clone()
                    } else {
                        Self::selection_accessibility_label(message)
                    },
                },
            );
        }

        let normalized_selected_ids: HashSet<String> = selected_message_ids
            .iter()
            .filter_map(|id| {
                let value = trimming_whitespaces_and_newlines(id);
                (!value.is_empty()).then(|| value.to_owned())
            })
            .collect();
        let toggle: Option<RiseManualListSelectionPresentationOnToggle> =
            on_toggle_message_selection.map(|callback| {
                Rc::new(move |selection_id: String, window: &mut Window, cx: &mut App| {
                    let message = messages_by_selection_id.get(&selection_id);
                    tracing::warn!(target: "riseonly::X1870", %selection_id, found = message.is_some(), "selection toggle routed");
                    if let Some(message) = message {
                        callback(message, window, cx);
                    }
                }) as RiseManualListSelectionPresentationOnToggle
            });
        self.list_view.update(cx, |list_view, cx| {
            list_view.apply_selection_presentation(
                RiseManualListSelectionPresentation {
                    is_active: true,
                    selected_ids: normalized_selected_ids,
                    items_by_item_id,
                    tint_color,
                    on_toggle: toggle,
                },
                window,
                cx,
            );
        });
    }

    pub(in super::super) fn selection_accessibility_label(message: &GetMessageMessage) -> String {
        let sender = trimming_whitespaces_and_newlines(&message.sender_name);
        let text = trimming_whitespaces_and_newlines(&message.text);
        let preview: String = if text.is_empty() {
            message.content_type.raw_value().to_owned()
        } else {
            text.chars().take(120).collect()
        };
        [sender.to_owned(), preview]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(". ")
    }

    pub fn apply(
        &mut self,
        new_configuration: RiseMessageCollectionConfiguration,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let new_configuration = Rc::new(new_configuration);
        if !self.initial_frame_ready_delivered {
            RiseColdOpenTrace::mark(|| {
                format!(
                    "list apply entries={} resolved={} src={} applying={} prep={}",
                    new_configuration.snapshot.entries.len(),
                    new_configuration.initial_data_resolved,
                    new_configuration.snapshot.source_version,
                    self.is_applying_snapshot,
                    self.native_preparation_task.is_some()
                )
            });
        }
        if self.parks_equivalent_pre_reveal_configuration(&new_configuration) {
            return;
        }
        let previous_configuration = self.configuration.clone();
        let changes_chat = self.presentation_chat_id.as_deref() != Some(new_configuration.chat_id.as_str());
        let changes_presentation = RiseMessageFirstFramePresentationPolicy::starts_new_presentation(
            self.presentation_epoch,
            new_configuration.presentation_epoch,
            changes_chat,
        );
        let active_account_user_id = SaiAccountCacheScope::active_user_id();
        let active_account_epoch = SaiAccountCacheScope::active_epoch();
        let changes_account = self.media_prefetch_account_user_id != active_account_user_id
            || self.media_prefetch_account_epoch != active_account_epoch;
        let changes_storage_key = previous_configuration
            .as_ref()
            .is_none_or(|previous| previous.scroll_storage_key() != new_configuration.scroll_storage_key());
        if changes_presentation || changes_account || changes_storage_key {
            self.cancel_directional_media_prefetch(cx);
            self.cancel_remote_page_prefetch(window, cx);
            self.media_prefetch_account_user_id = active_account_user_id;
            self.media_prefetch_account_epoch = active_account_epoch;
        }
        if changes_account && !changes_presentation {
            self.invalidate_history_edge_for_pending_presentation();
        }
        if changes_presentation || changes_account || changes_storage_key {
            self.last_reported_mark_candidate_ids.clear();
            self.deferred_mark_candidates.clear();
            self.deferred_mark_chat_id = None;
        }
        if changes_chat || changes_presentation || changes_storage_key {
            tracing::warn!(target: "riseonly::X696", "apply prev={:?} new={} changes_chat={changes_chat} changes_presentation={changes_presentation} changes_key={changes_storage_key} frame_ready={} restored={}", previous_configuration.as_ref().map(|previous| previous.scroll_storage_key()), new_configuration.scroll_storage_key(), self.initial_frame_ready_delivered, self.did_restore_initial_position);
        }
        if changes_presentation {
            if self.was_user_scrolling {
                let configuration = self
                    .rendered_configuration
                    .clone()
                    .or_else(|| previous_configuration.clone());
                self.deliver_scroll_activity_changed(false, configuration, window, cx);
                self.was_user_scrolling = false;
            }
            self.has_observed_history_scroll_interaction = false;
            if self.initial_frame_ready_delivered
                && self.did_restore_initial_position
                && let Some(previous_configuration) = &previous_configuration
            {
                self.persist_scroll_position(Some(previous_configuration), cx);
                ChatScrollPositionStore::flush_pending_to_disk(cx);
            }
            self.presentation_chat_id = Some(new_configuration.chat_id.clone());
            self.presentation_epoch = new_configuration.presentation_epoch;
            self.initial_frame_ready_delivered = false;
            self.initial_viewport_readiness_generation = self.initial_viewport_readiness_generation.wrapping_add(1);
            self.initial_viewport_readiness_pass_in_flight = false;
            self.initial_viewport_height_correction_pass = 0;
            self.initial_viewport_readiness_watchdog = None;
            self.initial_frame_owner = None;
            self.maintains_initial_bottom_pin = false;
            self.queued_pre_reveal_configuration = None;
            self.pending_initial_frame_reveal = None;
            self.cancel_deferred_history_preparation();
            self.did_restore_initial_position = false;
            self.pending_scroll_reset = false;
            self.pending_scroll_request = None;
            self.pending_reveal_highlight = None;
            self.history_edge_load_generation = self.history_edge_load_generation.wrapping_add(1);
            self.clear_pending_history_edge_load();
            self.history_edge_load_generation_by_source_version.clear();
            self.reset_viewport_state_delivery();
            self.load_older_armed = true;
            self.last_load_older_at = 0.0;
            self.load_newer_armed = true;
            self.last_load_newer_at = 0.0;
            self.list_view
                .update(cx, |list_view, _| list_view.cancel_visible_viewport_preparation());
            self.list_alpha = 0.0;
            cx.notify();
            self.unrendered_outgoing_ids.clear();
            ChatPerformanceTrace::shared().record(
                "first_frame.presentation_reset",
                HashMap::from([
                    ("chat_changed".to_owned(), if changes_chat { "1" } else { "0" }.to_owned()),
                    (
                        "epoch".to_owned(),
                        new_configuration
                            .presentation_epoch
                            .map(|epoch| epoch.uuid_string().chars().take(8).collect::<String>())
                            .unwrap_or_else(|| "none".to_owned()),
                    ),
                ]),
            );
        }

        if !self.initial_frame_ready_delivered
            && let Some(initial_frame_owner) = self.initial_frame_owner.clone()
            && initial_frame_owner.chat_id == new_configuration.chat_id
            && RiseMessageFirstFramePresentationPolicy::should_supersede_unrevealed_owner(
                initial_frame_owner.source_version,
                new_configuration.snapshot.source_version,
                new_configuration.initial_data_resolved,
            )
        {
            self.supersede_unrevealed_initial_frame_owner(&initial_frame_owner, &new_configuration, window, cx);
        }

        if let Some(last) = new_configuration.messages.last()
            && last.is_pending_outgoing()
            && MessageList::is_own_bubble(last, &new_configuration.chat, cx)
        {
            self.unrendered_outgoing_ids.insert(last.id.clone());
        }

        let was_near_bottom = self.is_near_bottom(cx);
        let chat_configuration_changed = previous_configuration.as_ref().is_none_or(|previous| {
            Self::cell_configuration_version(&previous.chat) != Self::cell_configuration_version(&new_configuration.chat)
        });
        let requested_scroll_request = new_configuration.scroll_request.clone().filter(|request| {
            previous_configuration
                .as_ref()
                .and_then(|previous| previous.scroll_request.as_ref())
                .is_none_or(|previous| previous.id != request.id)
        });
        let navigation_requested = requested_scroll_request.is_some();
        let scroll_reset_requested = requested_scroll_request
            .as_ref()
            .is_some_and(|request| request.destination != RiseMessageScrollDestination::Takeover);
        if navigation_requested {
            self.cancel_remote_page_prefetch(window, cx);
            self.maintains_initial_bottom_pin = false;
            let was_scrolling = self.was_user_scrolling;
            self.was_user_scrolling = false;
            self.reset_viewport_state_delivery();
            self.list_view
                .update(cx, |list_view, cx| list_view.stop_scrolling_for_immediate_transaction(window, cx));
            let destination = requested_scroll_request.as_ref().map(|request| &request.destination);
            if destination == Some(&RiseMessageScrollDestination::Takeover) {
                self.pending_scroll_reset = false;
                self.pending_scroll_request = None;
            }
            if was_scrolling {
                self.deliver_scroll_activity_changed(false, previous_configuration.clone(), window, cx);
            }
            if destination == Some(&RiseMessageScrollDestination::Latest) {
                ChatScrollPositionStore::clear(&new_configuration.scroll_storage_key(), cx);
            }
        }
        let source_version_changed = self.requested_source_version != Some(new_configuration.snapshot.source_version)
            || previous_configuration
                .as_ref()
                .is_none_or(|previous| previous.chat_id != new_configuration.chat_id);
        let data_resolution_became_available = !previous_configuration
            .as_ref()
            .is_some_and(|previous| previous.initial_data_resolved)
            && new_configuration.initial_data_resolved;
        let older_availability_changed = previous_configuration.as_ref().is_none_or(|previous| {
            RiseMessageHistoryEdgePolicy::did_availability_change(
                previous.can_load_older,
                new_configuration.can_load_older,
                previous.has_cached_older,
                new_configuration.has_cached_older,
            )
        });
        let newer_availability_changed = previous_configuration.as_ref().is_none_or(|previous| {
            RiseMessageHistoryEdgePolicy::did_availability_change(
                previous.can_load_newer,
                new_configuration.can_load_newer,
                previous.has_cached_newer,
                new_configuration.has_cached_newer,
            )
        });
        let interactions_changed = previous_configuration
            .as_ref()
            .is_none_or(|previous| previous.interactions_enabled != new_configuration.interactions_enabled);
        let selection_availability_changed = previous_configuration
            .as_ref()
            .is_none_or(|previous| previous.allows_message_selection != new_configuration.allows_message_selection);
        let requested_theme_palette = new_configuration.theme_palette.clone();
        let theme_changed = self.last_requested_theme_palette.as_ref() != Some(&requested_theme_palette);
        let render_environment_changed = previous_configuration
            .as_ref()
            .is_none_or(|previous| rise_message_rendered_environment_differs(previous, &new_configuration));
        self.last_requested_theme_palette = Some(requested_theme_palette);
        let new_source_version = new_configuration.snapshot.source_version;
        let presentation_needs_prepared_commit = changes_presentation
            && (self.applied_source_version != Some(new_source_version)
                || self
                    .rendered_configuration
                    .as_ref()
                    .is_none_or(|rendered| {
                        rendered.chat_id != new_configuration.chat_id
                            || rendered.snapshot.source_version != new_source_version
                    })
                || self.rendered_configuration_revision != self.configuration_revision
                || self.is_applying_snapshot);
        let adopts_equivalent_source_version = source_version_changed
            && self.initial_frame_ready_delivered
            && !changes_chat
            && !chat_configuration_changed
            && !theme_changed
            && !render_environment_changed
            && !data_resolution_became_available
            && !self.is_applying_snapshot
            && self.native_preparation_task.is_none()
            && self.prepared_snapshot_finalization_task.is_none()
            && self.pending_prepared_candidate.is_none()
            && self.rendered_configuration_revision == self.configuration_revision
            && self.rendered_configuration.as_ref().is_some_and(|rendered| {
                self.requested_source_version == Some(rendered.snapshot.source_version)
                    && self.applied_source_version == Some(rendered.snapshot.source_version)
                    && RiseMessageSnapshotReusePolicy::has_equivalent_rendered_content(
                        &rendered.snapshot,
                        &new_configuration.snapshot,
                    )
            });
        if adopts_equivalent_source_version {
            self.requested_source_version = Some(new_source_version);
            self.applied_source_version = Some(new_source_version);
            self.rendered_configuration = Some(new_configuration.clone());
            self.entries_by_id = new_configuration.snapshot.entries_by_id.clone();
            self.versions_by_id = new_configuration.snapshot.versions_by_id.clone();
            let versions_by_id = &new_configuration.snapshot.versions_by_id;
            self.pending_runtime_height_corrections.retain(|id, correction| {
                if versions_by_id.get(id) != Some(&correction.entry_content_version) {
                    return false;
                }
                correction.source_version = new_source_version;
                true
            });
            ChatPerformanceTrace::shared().record(
                "snapshot.source_adopt",
                HashMap::from([("entries".to_owned(), new_configuration.snapshot.entries.len().to_string())]),
            );
        }
        let source_changed = source_version_changed && !adopts_equivalent_source_version;
        let requires_prepared_commit = source_changed
            || chat_configuration_changed
            || theme_changed
            || render_environment_changed
            || data_resolution_became_available
            || presentation_needs_prepared_commit;
        if requires_prepared_commit {
            self.configuration_revision = self.configuration_revision.wrapping_add(1);
            tracing::warn!(
                target: "riseonly::X1793",
                source = new_source_version,
                entries = new_configuration.snapshot.entries.len(),
                chat_configuration_changed,
                revision = self.configuration_revision,
                ready = self.initial_frame_ready_delivered,
                "list apply"
            );
        }
        if new_configuration.snapshot.entries.len() <= 3 {
            tracing::warn!(
                target: "riseonly::X1711",
                chat = %new_configuration.chat_id,
                entries = new_configuration.snapshot.entries.len(),
                rendered_entries = self.rendered_configuration.as_ref().map(|rendered| rendered.snapshot.entries.len()),
                source_version_changed,
                adopts_equivalent_source_version,
                requires_prepared_commit,
                ready_delivered = self.initial_frame_ready_delivered,
                applying = self.is_applying_snapshot,
                preparing = self.native_preparation_task.is_some(),
                finalizing = self.prepared_snapshot_finalization_task.is_some(),
                candidate = self.pending_prepared_candidate.is_some(),
                alpha = self.list_alpha,
                "message list configuration"
            );
        }
        if chat_configuration_changed || theme_changed || render_environment_changed {
            self.item_environment_revision = self.item_environment_revision.wrapping_add(1);
        }

        self.configuration = Some(new_configuration.clone());
        self.list_view.update(cx, |list_view, cx| {
            list_view.set_is_scroll_enabled(new_configuration.is_scroll_enabled, cx);
        });
        self.apply_configured_viewport_insets(Some(&new_configuration), window, cx);
        let can_adopt_configuration_without_snapshot = !requires_prepared_commit
            && self.applied_source_version == Some(new_source_version)
            && self
                .rendered_configuration
                .as_ref()
                .is_some_and(|rendered| rendered.chat_id == new_configuration.chat_id)
            && self.rendered_configuration_revision == self.configuration_revision;
        if can_adopt_configuration_without_snapshot {
            self.rendered_configuration = Some(new_configuration.clone());
        }
        if interactions_changed || selection_availability_changed {
            self.presentation_revision = self.presentation_revision.wrapping_add(1);
        }
        if scroll_reset_requested {
            tracing::warn!(target: "riseonly::X696", "scroll reset drops restored key={} was={}", new_configuration.scroll_storage_key(), self.did_restore_initial_position);
            self.did_restore_initial_position = false;
            self.history_edge_load_generation = self.history_edge_load_generation.wrapping_add(1);
            self.clear_pending_history_edge_load();
            self.load_older_armed = true;
            self.last_load_older_at = 0.0;
            self.load_newer_armed = true;
            self.last_load_newer_at = 0.0;
            self.pending_scroll_reset = true;
            self.pending_scroll_request = requested_scroll_request.clone();
        }
        if newer_availability_changed {
            self.load_newer_armed = true;
            self.last_load_newer_at = 0.0;
            self.note_history_availability_change(
                RiseMessageManualListViewHistoryEdge::Newer,
                requires_prepared_commit && new_configuration.can_load_newer,
            );
        }
        if older_availability_changed {
            self.load_older_armed = true;
            self.last_load_older_at = 0.0;
            self.note_history_availability_change(
                RiseMessageManualListViewHistoryEdge::Older,
                requires_prepared_commit && new_configuration.can_load_older,
            );
        }
        if source_changed || chat_configuration_changed || render_environment_changed {
            self.pending_runtime_height_corrections.clear();
            self.runtime_height_correction_flush_scheduled = false;
        }

        if requires_prepared_commit && !self.initial_frame_ready_delivered {
            self.initial_viewport_readiness_generation = self.initial_viewport_readiness_generation.wrapping_add(1);
            self.initial_viewport_readiness_pass_in_flight = false;
            self.initial_viewport_readiness_watchdog = None;
            self.list_view
                .update(cx, |list_view, _| list_view.cancel_visible_viewport_preparation());
        }
        if requires_prepared_commit {
            self.cancel_deferred_history_preparation();
        }

        if !new_configuration.initial_data_resolved {
            self.initial_viewport_readiness_pass_in_flight = false;
            self.initial_viewport_readiness_watchdog = None;
            self.initial_frame_owner = None;
            self.maintains_initial_bottom_pin = false;
            self.queued_pre_reveal_configuration = None;
            self.cancel_deferred_history_preparation();
            self.requested_source_version = Some(new_source_version);
            self.set_pending_prepared_candidate(None, window, cx);
            self.candidate_being_predisplayed = None;
            self.native_preparation_generation = self.native_preparation_generation.wrapping_add(1);
            self.native_preparation_task = None;
            self.prepared_snapshot_finalization_generation =
                self.prepared_snapshot_finalization_generation.wrapping_add(1);
            self.prepared_snapshot_finalization_task = None;
            self.apply_generation = self.apply_generation.wrapping_add(1);
            self.list_view.update(cx, |list_view, cx| list_view.stop(window, cx));
            self.is_applying_snapshot = false;
            self.clear_pending_presentation_commit(&new_configuration, true);
            return;
        }

        if !requires_prepared_commit {
            if can_adopt_configuration_without_snapshot {
                if interactions_changed || selection_availability_changed {
                    let anchor = if self.maintains_initial_bottom_pin || was_near_bottom {
                        RiseManualListTransactionAnchor::Bottom
                    } else {
                        RiseManualListTransactionAnchor::PreserveVisible
                    };
                    self.enqueue_rendered_items(&new_configuration, anchor, window, cx);
                }
                self.clear_pending_presentation_commit(&new_configuration, true);
            }
            if scroll_reset_requested {
                self.consume_pending_scroll_request(&new_configuration, window, cx);
            }
            if older_availability_changed || newer_availability_changed {
                let metrics = self.list_view.read(cx).scroll_metrics();
                self.scroll_metrics_did_change(&metrics, window, cx);
            }
            self.flush_deferred_work_if_possible(window, cx);
            self.schedule_initial_frame_readiness_if_needed(window, cx);
            return;
        }

        self.requested_source_version = Some(new_source_version);
        if !self.initial_frame_ready_delivered && self.initial_frame_owner.is_none() {
            let has_saved_position = new_configuration.saved_scroll_position(cx).is_some()
                || new_configuration
                    .snapshot
                    .entries
                    .iter()
                    .any(|entry| entry.kind == ChatMessageRenderEntryKind::UnreadSeparator);
            let pins_initial_position_to_bottom =
                RiseMessageFirstFramePresentationPolicy::pins_initial_position_to_bottom(
                    new_configuration.initial_scroll_position,
                    has_saved_position,
                );
            self.initial_frame_owner = Some(RiseMessageManualListViewInitialFrameOwner {
                chat_id: new_configuration.chat_id.clone(),
                source_version: new_source_version,
                entry_count: new_configuration.snapshot.entries.len(),
                pins_initial_position_to_bottom,
            });
            self.maintains_initial_bottom_pin = pins_initial_position_to_bottom;
        }
        self.set_pending_prepared_candidate(None, window, cx);
        self.prepared_snapshot_finalization_generation =
            self.prepared_snapshot_finalization_generation.wrapping_add(1);
        self.prepared_snapshot_finalization_task = None;
        self.candidate_being_predisplayed = None;
        let prepared_history_edge_load_generation = if source_version_changed
            || older_availability_changed
            || newer_availability_changed
        {
            match self
                .pending_history_edge_load
                .as_ref()
                .filter(|pending| pending.chat_id == new_configuration.chat_id)
            {
                Some(pending) => {
                    let originating_generation = self
                        .history_edge_load_generation_by_source_version
                        .get(&new_source_version)
                        .copied()
                        .unwrap_or(pending.generation);
                    self.history_edge_load_generation_by_source_version.clear();
                    self.history_edge_load_generation_by_source_version
                        .insert(new_source_version, originating_generation);
                    Some(originating_generation)
                }
                None => None,
            }
        } else {
            None
        };
        let prepared_configuration = new_configuration.clone();
        let trace_entries = new_configuration.snapshot.entries.len();
        if !self.initial_frame_ready_delivered {
            RiseColdOpenTrace::mark(|| format!("list layouts begin entries={trace_entries}"));
        }
        self.prepare_native_layouts(
            new_configuration,
            Box::new(move |native_batch, this, window, cx| {
                if !this.initial_frame_ready_delivered {
                    RiseColdOpenTrace::mark(|| format!("list layouts done entries={trace_entries}"));
                }
                this.enqueue_prepared_snapshot(
                    prepared_configuration,
                    native_batch,
                    HashMap::new(),
                    None,
                    prepared_history_edge_load_generation,
                    window,
                    cx,
                );
            }),
            window,
            cx,
        );
    }

    /// a pending load whose edge changed availability
    /// a commit still to come reports progress, otherwise nothing will and the request ends here
    pub(in super::super) fn note_history_availability_change(
        &mut self,
        edge: RiseMessageManualListViewHistoryEdge,
        records_progress: bool,
    ) {
        let Some(pending) = self.pending_history_edge_load.as_ref().filter(|pending| pending.edge == edge) else {
            return;
        };
        if records_progress {
            self.pending_history_edge_progress_generation = Some(pending.generation);
        } else {
            self.clear_pending_history_edge_load();
        }
    }

    pub(in super::super) fn parks_equivalent_pre_reveal_configuration(
        &mut self,
        incoming: &Rc<RiseMessageCollectionConfiguration>,
    ) -> bool {
        if self.initial_frame_ready_delivered {
            return false;
        }
        let (Some(current), Some(owner)) = (self.configuration.as_ref(), self.initial_frame_owner.as_ref()) else {
            return false;
        };
        let owner_awaits_reveal = self.applied_source_version == Some(owner.source_version);
        let preparation_in_flight = self.is_applying_snapshot
            || self.native_preparation_task.is_some()
            || self.pending_native_configuration.is_some()
            || self.prepared_snapshot_finalization_task.is_some()
            || self.pending_prepared_candidate.is_some()
            || owner_awaits_reveal;
        let parks = preparation_in_flight
            && incoming.initial_data_resolved
            && current.initial_data_resolved
            && owner.chat_id == incoming.chat_id
            && current.chat_id == incoming.chat_id
            && owner.source_version == current.snapshot.source_version
            && incoming.snapshot.source_version != current.snapshot.source_version
            && self.requested_source_version == Some(current.snapshot.source_version)
            && current.presentation_epoch == incoming.presentation_epoch
            && self.media_prefetch_account_user_id == SaiAccountCacheScope::active_user_id()
            && self.media_prefetch_account_epoch == SaiAccountCacheScope::active_epoch()
            && current.scroll_storage_key() == incoming.scroll_storage_key()
            && current.scroll_request.as_ref().map(|request| request.id)
                == incoming.scroll_request.as_ref().map(|request| request.id)
            && current.initial_scroll_position == incoming.initial_scroll_position
            && current.top_inset == incoming.top_inset
            && current.bottom_inset == incoming.bottom_inset
            && current.sticky_bottom_inset == incoming.sticky_bottom_inset
            && current.keyboard_resting_bottom_inset == incoming.keyboard_resting_bottom_inset
            && current.keyboard_resting_sticky_bottom_inset == incoming.keyboard_resting_sticky_bottom_inset
            && current.keyboard_active_bottom_inset == incoming.keyboard_active_bottom_inset
            && current.keyboard_active_sticky_bottom_inset == incoming.keyboard_active_sticky_bottom_inset
            && current.tracks_system_keyboard == incoming.tracks_system_keyboard
            && current.is_scroll_enabled == incoming.is_scroll_enabled
            && current.interactions_enabled == incoming.interactions_enabled
            && current.allows_message_selection == incoming.allows_message_selection
            && current.can_load_older == incoming.can_load_older
            && current.can_load_newer == incoming.can_load_newer
            && current.has_cached_older == incoming.has_cached_older
            && current.has_cached_newer == incoming.has_cached_newer
            && current.search_highlight == incoming.search_highlight
            && self.last_requested_theme_palette.as_ref() == Some(&incoming.theme_palette)
            && !rise_message_rendered_environment_differs(current, incoming)
            && Self::cell_configuration_version(&current.chat) == Self::cell_configuration_version(&incoming.chat)
            && RiseMessageSnapshotReusePolicy::has_equivalent_rendered_content(&current.snapshot, &incoming.snapshot);
        if parks {
            self.queued_pre_reveal_configuration = Some(incoming.clone());
        }
        parks
    }

    pub(in super::super) fn supersede_unrevealed_initial_frame_owner(
        &mut self,
        owner: &RiseMessageManualListViewInitialFrameOwner,
        incoming_configuration: &RiseMessageCollectionConfiguration,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        ChatPerformanceTrace::shared().record(
            "first_frame.owner_superseded",
            HashMap::from([
                ("owner_source".to_owned(), owner.source_version.to_string()),
                ("owner_entries".to_owned(), owner.entry_count.to_string()),
                ("incoming_source".to_owned(), incoming_configuration.snapshot.source_version.to_string()),
                ("incoming_entries".to_owned(), incoming_configuration.snapshot.entries.len().to_string()),
            ]),
        );
        MessagePaginationTrace::record(
            "first_frame.owner_superseded",
            &incoming_configuration.chat_id,
            None,
            None,
            None,
            None,
            None,
            Some(incoming_configuration.snapshot.entries.len() as i64),
            Some(owner.entry_count as i64),
            None,
            Some(&format!(
                "ownerSource={} incomingSource={}",
                owner.source_version, incoming_configuration.snapshot.source_version
            )),
            cx,
        );

        self.initial_viewport_readiness_generation = self.initial_viewport_readiness_generation.wrapping_add(1);
        self.initial_viewport_readiness_pass_in_flight = false;
        self.initial_viewport_height_correction_pass = 0;
        self.initial_viewport_readiness_watchdog = None;
        self.initial_frame_owner = None;
        self.maintains_initial_bottom_pin = false;
        self.queued_pre_reveal_configuration = None;
        self.pending_initial_frame_reveal = None;
        self.cancel_deferred_history_preparation();
        self.set_pending_prepared_candidate(None, window, cx);
        self.prepared_snapshot_finalization_generation =
            self.prepared_snapshot_finalization_generation.wrapping_add(1);
        self.prepared_snapshot_finalization_task = None;
        self.candidate_being_predisplayed = None;
        self.native_preparation_generation = self.native_preparation_generation.wrapping_add(1);
        self.native_preparation_task = None;
        self.apply_generation = self.apply_generation.wrapping_add(1);
        self.list_view.update(cx, |list_view, cx| {
            list_view.stop(window, cx);
            list_view.cancel_visible_viewport_preparation();
        });
        self.is_applying_snapshot = false;
        self.list_alpha = 0.0;
        cx.notify();
    }

    #[allow(clippy::too_many_arguments)]
    pub(in super::super) fn enqueue_prepared_snapshot(
        &mut self,
        prepared_configuration: Rc<RiseMessageCollectionConfiguration>,
        native_batch: RiseMessageManualListViewPreparedNativeBatch,
        fallback_heights: HashMap<String, f32>,
        prepared_revision: Option<u64>,
        history_edge_load_generation: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let width = f32::from(self.bounds.size.width);
        if self.requested_source_version != Some(prepared_configuration.snapshot.source_version)
            || self
                .configuration
                .as_ref()
                .is_none_or(|configuration| configuration.chat_id != prepared_configuration.chat_id)
            || native_batch.source_version != prepared_configuration.snapshot.source_version
            || native_batch.chat_id != prepared_configuration.chat_id
            || native_batch.configuration_revision != self.configuration_revision
            || (native_batch.width - width).abs() > 0.5
        {
            tracing::warn!(
                target: "riseonly::X1793",
                requested = ?self.requested_source_version,
                prepared = prepared_configuration.snapshot.source_version,
                batch = native_batch.source_version,
                batch_revision = native_batch.configuration_revision,
                revision = self.configuration_revision,
                batch_width = native_batch.width,
                width,
                "enqueue dropped"
            );
            return;
        }
        let latest_configuration = self
            .configuration
            .as_ref()
            .filter(|configuration| configuration.snapshot.source_version == prepared_configuration.snapshot.source_version)
            .cloned()
            .unwrap_or(prepared_configuration);
        let mut merged_native_batch = native_batch;
        if self
            .rendered_configuration
            .as_ref()
            .is_some_and(|rendered| rendered.chat_id == latest_configuration.chat_id)
            && self.rendered_configuration_revision == merged_native_batch.configuration_revision
            && (self.rendered_preparation_width - merged_native_batch.width).abs() <= 0.5
        {
            let expected_versions = &latest_configuration.snapshot.versions_by_id;
            for (id, prepared) in &self.hosted_text {
                if !merged_native_batch.hosted_text.contains_key(id)
                    && expected_versions.contains_key(id)
                    && self.versions_by_id.get(id) == expected_versions.get(id)
                {
                    merged_native_batch.hosted_text.insert(id.clone(), prepared.clone());
                }
            }
            for (id, layout) in &self.native_layouts {
                if merged_native_batch.layouts.contains_key(id) {
                    continue;
                }
                let Some(expected_version) = expected_versions.get(id) else {
                    continue;
                };
                if self.native_layout_versions.get(id) != Some(expected_version) {
                    continue;
                }
                merged_native_batch.layouts.insert(id.clone(), layout.clone());
                merged_native_batch.versions.insert(id.clone(), *expected_version);
                if let Some(ownership) = self.native_ownership_by_id.get(id) {
                    merged_native_batch.ownership_by_id.insert(id.clone(), *ownership);
                }
            }
        }
        let mut candidate_fallback_heights =
            if prepared_revision.is_none_or(|revision| revision == self.configuration_revision) {
                fallback_heights
            } else {
                HashMap::new()
            };
        if self.can_reuse_measured_fallback_heights(&latest_configuration, merged_native_batch.width) {
            for entry in &latest_configuration.snapshot.entries {
                if self.versions_by_id.get(&entry.id) == Some(&entry.content_version)
                    && !candidate_fallback_heights.contains_key(&entry.id)
                    && let Some(measured_height) = self.measured_heights.get(&entry.id)
                {
                    candidate_fallback_heights.insert(entry.id.clone(), *measured_height);
                }
            }
        }
        let candidate = RiseMessageManualListViewPreparedSnapshotCandidate {
            configuration: latest_configuration,
            configuration_revision: merged_native_batch.configuration_revision,
            native_batch: merged_native_batch,
            fallback_heights: candidate_fallback_heights,
            history_edge_load_generation,
        };
        if self.is_applying_snapshot {
            tracing::warn!(
                target: "riseonly::X2173",
                chat = %candidate.configuration.chat_id,
                entries = candidate.configuration.snapshot.entries.len(),
                "enqueue parked by apply"
            );
            self.set_pending_prepared_candidate(Some(candidate), window, cx);
            return;
        }
        self.set_pending_prepared_candidate(None, window, cx);
        self.finalize_prepared_snapshot(candidate, window, cx);
    }

    /// requires_prepared_geometry false asks only if the page could go in once measured
    /// the commit itself always rechecks with exact geometry
    pub(in super::super) fn scroll_safe_edge_commit(
        &self,
        candidate: &RiseMessageManualListViewPreparedSnapshotCandidate,
        requires_prepared_geometry: bool,
        cx: &App,
    ) -> Option<RiseMessageManualListViewScrollSafeEdgeCommit> {
        let list_view = self.list_view.read(cx);
        let metrics = list_view.scroll_metrics();
        if !(metrics.is_tracking || metrics.is_dragging || metrics.is_decelerating)
            || !self.initial_frame_ready_delivered
            || !self.did_restore_initial_position
            || self.pending_scroll_reset
            || candidate.native_batch.purpose != RiseMessageManualListViewNativePreparationPurpose::Foreground
        {
            return None;
        }
        let previous_configuration = self.rendered_configuration.as_ref()?;
        if previous_configuration.chat_id != candidate.configuration.chat_id
            || self.applied_source_version != Some(previous_configuration.snapshot.source_version)
            || (self.rendered_preparation_width - candidate.native_batch.width).abs() > 0.5
            || !Self::scroll_safe_environment_is_unchanged(previous_configuration, &candidate.configuration)
        {
            return None;
        }

        let old_ids = &previous_configuration.snapshot.ids;
        let new_ids = &candidate.configuration.snapshot.ids;
        if !RiseMessagePreparedGeometryPolicy::identities_can_form_scroll_safe_mutation(old_ids, new_ids) {
            return None;
        }
        let mutation = Self::edge_mutation(old_ids, new_ids)?;

        let old_lower_bound = mutation.removed_prefix_count;
        let old_upper_bound = old_ids.len().checked_sub(mutation.removed_suffix_count)?;
        let new_lower_bound = mutation.inserted_prefix_count;
        let new_upper_bound = new_ids.len().checked_sub(mutation.inserted_suffix_count)?;
        if old_lower_bound >= old_upper_bound || new_lower_bound >= new_upper_bound {
            return None;
        }

        let old_retained_ids = &old_ids[old_lower_bound..old_upper_bound];
        if old_retained_ids != &new_ids[new_lower_bound..new_upper_bound] {
            return None;
        }
        let retained_ids: HashSet<String> = old_retained_ids.iter().cloned().collect();
        let new_id_set: HashSet<&String> = new_ids.iter().collect();
        let removed_edge_ids: HashSet<String> = old_ids[..old_lower_bound]
            .iter()
            .chain(old_ids[old_upper_bound..].iter())
            .cloned()
            .collect();
        let inserted_edge_ids: HashSet<String> = new_ids[..new_lower_bound]
            .iter()
            .chain(new_ids[new_upper_bound..].iter())
            .cloned()
            .collect();
        let common_edge_ids_are_edges = old_ids
            .iter()
            .filter(|id| new_id_set.contains(id) && !retained_ids.contains(*id))
            .all(|id| removed_edge_ids.contains(id) && inserted_edge_ids.contains(id));
        if !common_edge_ids_are_edges {
            return None;
        }

        let exact_geometry_is_prepared = !requires_prepared_geometry
            || self.scroll_safe_geometry_is_complete(candidate, &mutation, None);

        let visible_ids: HashSet<String> = list_view.visible_ids().iter().cloned().collect();
        let removable_visible_ids = Self::removable_decoration_ids(&removed_edge_ids, previous_configuration);
        let visible_items_are_retained = RiseMessageActiveScrollEdgeCommitPolicy::visible_items_survive_mutation(
            &visible_ids,
            &retained_ids,
            &removable_visible_ids,
        );
        if !RiseMessageActiveScrollEdgeCommitPolicy::allows_commit(
            metrics.is_tracking,
            metrics.is_dragging,
            metrics.is_decelerating,
            true,
            visible_items_are_retained,
            exact_geometry_is_prepared,
        ) {
            return None;
        }
        for id in &retained_ids {
            let previous_version = self.versions_by_id.get(id)?;
            let next_version = candidate.configuration.snapshot.versions_by_id.get(id)?;
            if !self.manual_items_by_id.contains_key(id) {
                return None;
            }
            let index = *candidate.configuration.snapshot.indices_by_id.get(id)?;
            let entry = &candidate.configuration.snapshot.entries[index];
            if previous_version != next_version {
                if entry.kind == ChatMessageRenderEntryKind::Message {
                    let has_prepared_native_geometry = candidate.native_batch.versions.get(id) == Some(next_version)
                        && candidate.native_batch.layouts.contains_key(id)
                        && candidate.native_batch.ownership_by_id.contains_key(id);
                    let has_exact_fallback_geometry = RiseMessagePreparedGeometryPolicy::has_exact_fallback_height(
                        candidate.fallback_heights.get(id).copied(),
                    );
                    if !has_prepared_native_geometry && !has_exact_fallback_geometry && requires_prepared_geometry {
                        return None;
                    }
                }
                continue;
            }
            if self.rendered_native_ids.contains(id)
                && (self.native_layout_versions.get(id) != Some(next_version)
                    || !self.native_layouts.contains_key(id)
                    || !self.native_ownership_by_id.contains_key(id))
            {
                return None;
            }
        }
        Some(RiseMessageManualListViewScrollSafeEdgeCommit {
            mutation,
            retained_ids,
            removable_visible_ids,
        })
    }

    /// separators the mutation drops
    /// a window opens with the date row of its first message and a page of the same day dissolves it
    pub(in super::super) fn removable_decoration_ids(
        removed_ids: &HashSet<String>,
        configuration: &RiseMessageCollectionConfiguration,
    ) -> HashSet<String> {
        removed_ids
            .iter()
            .filter(|id| {
                configuration.snapshot.entries_by_id.get(*id).is_some_and(|entry| {
                    matches!(
                        entry.kind,
                        ChatMessageRenderEntryKind::DateSeparator
                            | ChatMessageRenderEntryKind::TopicSeparator
                            | ChatMessageRenderEntryKind::UnreadSeparator
                    )
                })
            })
            .cloned()
            .collect()
    }

    pub(in super::super) fn trace_deferred_render(
        &self,
        candidate: &RiseMessageManualListViewPreparedSnapshotCandidate,
        reason: &str,
        cx: &mut App,
    ) {
        let metrics = self.list_view.read(cx).scroll_metrics();
        tracing::warn!(
            target: "riseonly::X2173",
            chat = %candidate.configuration.chat_id,
            entries = candidate.configuration.snapshot.entries.len(),
            reason,
            tracking = metrics.is_tracking,
            dragging = metrics.is_dragging,
            decelerating = metrics.is_decelerating,
            "commit deferred"
        );
        let bit = |value: bool| if value { "1" } else { "0" }.to_owned();
        ChatPerformanceTrace::shared().record(
            "snapshot.defer",
            HashMap::from([
                ("reason".to_owned(), reason.to_owned()),
                ("entries".to_owned(), candidate.configuration.snapshot.entries.len().to_string()),
                ("tracking".to_owned(), bit(metrics.is_tracking)),
                ("dragging".to_owned(), bit(metrics.is_dragging)),
                ("decelerating".to_owned(), bit(metrics.is_decelerating)),
            ]),
        );
        MessagePaginationTrace::record(
            "render.defer",
            &candidate.configuration.chat_id,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(candidate.configuration.snapshot.entries.len() as i64),
            None,
            Some(&format!(
                "reason={reason} tracking={} dragging={} decelerating={}",
                metrics.is_tracking, metrics.is_dragging, metrics.is_decelerating
            )),
            cx,
        );
    }

    pub(in super::super) fn merge_version_matched_retained_geometry(
        &self,
        candidate: &mut RiseMessageManualListViewPreparedSnapshotCandidate,
        retained_ids: &HashSet<String>,
        configuration: &RiseMessageCollectionConfiguration,
    ) {
        for id in retained_ids {
            let Some(next_version) = configuration.snapshot.versions_by_id.get(id).copied() else {
                continue;
            };
            let previous_version = self.versions_by_id.get(id).copied();
            let previous_height = self.measured_heights.get(id).copied();
            if RiseMessagePreparedGeometryPolicy::should_adopt_previous_fallback_height(
                previous_version,
                next_version,
                candidate.fallback_heights.get(id).copied(),
                previous_height,
            ) {
                match previous_height {
                    Some(height) => candidate.fallback_heights.insert(id.clone(), height),
                    None => candidate.fallback_heights.remove(id),
                };
            }

            if !self.rendered_native_ids.contains(id) || self.native_layout_versions.get(id) != Some(&next_version) {
                continue;
            }
            let (Some(layout), Some(ownership)) =
                (self.native_layouts.get(id), self.native_ownership_by_id.get(id))
            else {
                continue;
            };
            candidate.native_batch.layouts.insert(id.clone(), layout.clone());
            candidate.native_batch.versions.insert(id.clone(), next_version);
            candidate.native_batch.ownership_by_id.insert(id.clone(), *ownership);
        }
    }

    pub(in super::super) fn commit_prepared_snapshot(
        &mut self,
        prepared_candidate: RiseMessageManualListViewPreparedSnapshotCandidate,
        requested_scroll_safe_edge_commit: Option<RiseMessageManualListViewScrollSafeEdgeCommit>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let trace_token = ChatPerformanceTrace::shared().begin_operation(
            "snapshot.commit_prepare",
            HashMap::from([
                ("entries".to_owned(), prepared_candidate.configuration.snapshot.entries.len().to_string()),
                (
                    "purpose".to_owned(),
                    Self::preparation_purpose_trace_name(prepared_candidate.native_batch.purpose).to_owned(),
                ),
            ]),
        );
        self.commit_prepared_snapshot_inner(prepared_candidate, requested_scroll_safe_edge_commit, window, cx);
        ChatPerformanceTrace::shared().end_operation(trace_token, 2.0, HashMap::new());
    }

    pub(in super::super) fn commit_prepared_snapshot_inner(
        &mut self,
        prepared_candidate: RiseMessageManualListViewPreparedSnapshotCandidate,
        requested_scroll_safe_edge_commit: Option<RiseMessageManualListViewScrollSafeEdgeCommit>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut candidate = prepared_candidate;
        let new_configuration = candidate.configuration.clone();
        if self.requested_source_version != Some(new_configuration.snapshot.source_version)
            || self.configuration.as_ref().map(|configuration| configuration.presentation_epoch)
                != Some(new_configuration.presentation_epoch)
            || candidate.configuration_revision != self.configuration_revision
            || self.is_applying_snapshot
        {
            tracing::warn!(
                target: "riseonly::X1793",
                requested = ?self.requested_source_version,
                candidate = new_configuration.snapshot.source_version,
                candidate_revision = candidate.configuration_revision,
                revision = self.configuration_revision,
                applying = self.is_applying_snapshot,
                "commit dropped"
            );
            return;
        }
        if (candidate.native_batch.width - f32::from(self.bounds.size.width)).abs() > 0.5 {
            tracing::warn!(
                target: "riseonly::X2173",
                chat = %new_configuration.chat_id,
                entries = new_configuration.snapshot.entries.len(),
                batch_width = candidate.native_batch.width,
                width = f32::from(self.bounds.size.width),
                "commit dropped by width"
            );
            self.native_preparation_width = 0.0;
            self.pending_width_repreparation = true;
            cx.defer_in(window, |this, window, cx| this.flush_deferred_work_if_possible(window, cx));
            return;
        }

        if self.is_travelling(cx) && !self.is_user_scrolling(cx) {
            tracing::warn!(target: "riseonly::X1793", source = new_configuration.snapshot.source_version, "commit parked by travel");
            self.trace_deferred_render(&candidate, "programmatic-travel", cx);
            self.set_pending_prepared_candidate(Some(candidate), window, cx);
            return;
        }
        let scroll_safe_edge_commit = if self.is_user_scrolling(cx) {
            let Some(requested) = requested_scroll_safe_edge_commit else {
                self.trace_deferred_render(&candidate, "edge-not-scroll-safe", cx);
                self.set_pending_prepared_candidate(Some(candidate), window, cx);
                return;
            };
            self.merge_version_matched_retained_geometry(&mut candidate, &requested.retained_ids, &new_configuration);
            let current = self
                .scroll_safe_edge_commit(&candidate, true, cx)
                .filter(|current| {
                    current.mutation == requested.mutation && current.retained_ids == requested.retained_ids
                });
            let Some(current) = current else {
                self.trace_deferred_render(&candidate, "edge-not-scroll-safe", cx);
                self.set_pending_prepared_candidate(Some(candidate), window, cx);
                return;
            };
            if self.holds_edge_page_for_fast_fling(cx) {
                self.trace_deferred_render(&candidate, "fast-fling", cx);
                self.set_pending_prepared_candidate(Some(candidate), window, cx);
                return;
            }
            Some(current)
        } else {
            None
        };

        let prepared_range = &candidate.native_batch.entry_range;
        let exact_geometry_range = RiseFallbackMeasurementPolicy::exact_geometry_range_for_commit(
            new_configuration.snapshot.entries.len() as i64,
            prepared_range.start as i64..prepared_range.end as i64,
            self.initial_frame_ready_delivered,
            candidate.native_batch.purpose == RiseMessageManualListViewNativePreparationPurpose::DeferredHistory,
        );
        let unresolved_geometry = match &scroll_safe_edge_commit {
            Some(commit) => self.unresolved_fallback_geometry_count_at_indices(
                &candidate,
                &self.scroll_safe_fallback_geometry_indices(&candidate, &commit.mutation),
                &commit.retained_ids,
            ),
            None => self.unresolved_fallback_geometry_count(
                &candidate,
                exact_geometry_range.start as usize..exact_geometry_range.end as usize,
            ),
        };
        if unresolved_geometry != 0 {
            tracing::warn!(target: "riseonly::X1793", source = new_configuration.snapshot.source_version, unresolved_geometry, "commit parked by geometry");
            ChatPerformanceTrace::shared().record(
                "geometry.blocked_commit",
                HashMap::from([
                    ("unresolved".to_owned(), unresolved_geometry.to_string()),
                    ("entries".to_owned(), new_configuration.snapshot.entries.len().to_string()),
                    (
                        "required_range".to_owned(),
                        format!("{}..<{}", exact_geometry_range.start, exact_geometry_range.end),
                    ),
                    (
                        "purpose".to_owned(),
                        Self::preparation_purpose_trace_name(candidate.native_batch.purpose).to_owned(),
                    ),
                    (
                        "scroll_safe".to_owned(),
                        if scroll_safe_edge_commit.is_none() { "0" } else { "1" }.to_owned(),
                    ),
                ]),
            );
            debug_assert!(
                scroll_safe_edge_commit.is_some(),
                "A completed message snapshot attempted to commit unresolved geometry"
            );
            self.set_pending_prepared_candidate(Some(candidate), window, cx);
            return;
        }

        let previous_configuration = self.rendered_configuration.clone();
        let previous_last_message = previous_configuration
            .as_ref()
            .and_then(|previous| previous.messages.last().cloned());
        let previous_first_message_id = previous_configuration
            .as_ref()
            .and_then(|previous| previous.messages.first().map(|message| message.id.clone()));
        let was_near_bottom = self.is_near_bottom(cx);
        let was_at_exact_bottom = self.is_at_exact_bottom(cx);
        let stationary_anchor = self.preferred_visible_anchor(cx);
        let is_different_chat = previous_configuration.as_ref().is_none_or(|previous| {
            previous.chat_id != new_configuration.chat_id
                || previous.scroll_storage_key() != new_configuration.scroll_storage_key()
        });
        if is_different_chat && let Some(previous_configuration) = &previous_configuration {
            self.persist_scroll_position(Some(previous_configuration), cx);
            ChatScrollPositionStore::flush_pending_to_disk(cx);
            self.did_restore_initial_position = false;
            self.load_older_armed = true;
            self.last_load_older_at = 0.0;
            self.measured_heights.clear();
        }

        let new_first_message_id = new_configuration.messages.first().map(|message| &message.id);
        let did_prepend = previous_first_message_id.as_ref().is_some_and(|previous_first| {
            new_first_message_id != Some(previous_first)
                && new_configuration.messages.iter().any(|message| &message.id == previous_first)
        });
        let new_last_message = new_configuration.messages.last().cloned();
        let new_last_id = new_last_message.as_ref().map(|message| &message.id);
        let appended_message = previous_last_message.as_ref().map(|message| &message.id) != new_last_id;
        let did_append = previous_last_message.as_ref().is_some_and(|previous| {
            Some(&previous.id) != new_last_id
                && new_configuration.messages.iter().any(|message| message.id == previous.id)
        });
        let new_last_is_own = new_last_message
            .as_ref()
            .map(|message| MessageList::is_own_bubble(message, &new_configuration.chat, cx));
        let sent_own_message = appended_message
            && new_last_is_own == Some(true)
            && new_last_message.as_ref().is_some_and(|message| {
                message.is_pending_outgoing() || self.unrendered_outgoing_ids.contains(&message.id)
            });
        let follows_incoming = appended_message && new_last_is_own == Some(false) && was_near_bottom;

        let next_native_ids: HashSet<String> = new_configuration
            .snapshot
            .entries
            .iter()
            .filter(|entry| {
                candidate.native_batch.versions.get(&entry.id) == Some(&entry.content_version)
                    && candidate.native_batch.layouts.contains_key(&entry.id)
                    && candidate.native_batch.ownership_by_id.contains_key(&entry.id)
            })
            .map(|entry| entry.id.clone())
            .collect();
        if let Some(commit) = &scroll_safe_edge_commit
            && !self.scroll_safe_geometry_is_complete(&candidate, &commit.mutation, Some(&next_native_ids))
        {
            self.trace_deferred_render(&candidate, "resolved-geometry-incomplete", cx);
            self.set_pending_prepared_candidate(Some(candidate), window, cx);
            return;
        }
        for message in new_configuration.messages.iter() {
            if self.unrendered_outgoing_ids.is_empty() {
                break;
            }
            self.unrendered_outgoing_ids.remove(&message.id);
        }
        let empty_retained_ids = HashSet::new();
        let retained_item_ids = scroll_safe_edge_commit
            .as_ref()
            .map(|commit| &commit.retained_ids)
            .unwrap_or(&empty_retained_ids);
        let item_table = self.make_manual_items(&candidate, &next_native_ids, retained_item_ids, window, cx);
        let new_ids = new_configuration.snapshot.ids.clone();
        let empty_ids = Vec::new();
        let previous_ids = previous_configuration
            .as_ref()
            .map(|previous| &previous.snapshot.ids)
            .unwrap_or(&empty_ids);
        let edge_mutation = scroll_safe_edge_commit
            .as_ref()
            .map(|commit| commit.mutation)
            .or_else(|| Self::edge_mutation(previous_ids, &new_ids));
        let reaction_update_ids = Self::reaction_update_ids(previous_configuration.as_deref(), &new_configuration);
        let new_id_set: HashSet<String> = new_ids.iter().cloned().collect();
        let reset_requested = self.pending_scroll_reset || !self.did_restore_initial_position;
        let previous_id_set: HashSet<String> = previous_ids.iter().cloned().collect();
        let is_quiet_commit = scroll_safe_edge_commit.is_none()
            && self.initial_frame_ready_delivered
            && !is_different_chat
            && !reset_requested;
        let travels_to_own_message =
            is_quiet_commit && sent_own_message && !was_near_bottom && !new_configuration.can_load_newer;
        let inserted_message_ids: HashSet<String> =
            if is_quiet_commit && did_append && (sent_own_message || follows_incoming || was_near_bottom) {
                let mut inserted = HashSet::new();
                let mut pending_separators: Vec<String> = Vec::new();
                for entry in &new_configuration.snapshot.entries {
                    match entry.kind {
                        ChatMessageRenderEntryKind::DateSeparator
                        | ChatMessageRenderEntryKind::TopicSeparator
                        | ChatMessageRenderEntryKind::UnreadSeparator => {
                            if !previous_id_set.contains(&entry.id) {
                                pending_separators.push(entry.id.clone());
                            }
                        }
                        ChatMessageRenderEntryKind::System => pending_separators.clear(),
                        ChatMessageRenderEntryKind::Message => {
                            if !previous_id_set.contains(&entry.id) {
                                inserted.insert(entry.id.clone());
                                inserted.extend(pending_separators.drain(..));
                            }
                            pending_separators.clear();
                        }
                    }
                }
                inserted
            } else {
                HashSet::new()
            };
        let backfill_ids: HashSet<String> = new_id_set.difference(&previous_id_set).cloned().collect();
        let older_edge_changed_here = previous_configuration.as_ref().is_some_and(|previous| {
            previous.chat_id == new_configuration.chat_id
                && (previous.can_load_older != new_configuration.can_load_older
                    || previous.has_cached_older != new_configuration.has_cached_older)
        });
        let is_older_page_in_flight = older_edge_changed_here
            || self.pending_history_edge_load.as_ref().is_some_and(|pending| {
                pending.edge == RiseMessageManualListViewHistoryEdge::Older && pending.chat_id == new_configuration.chat_id
            });
        let is_top_backfill_only = !is_older_page_in_flight
            && Self::is_top_backfill_only(
                &backfill_ids,
                &new_ids,
                &previous_id_set,
                previous_id_set.difference(&new_id_set).count(),
                MessagePresentationWindowProjector::stride() as usize,
            );
        let removed_message_ids: HashSet<String> = if is_quiet_commit
            && (!did_prepend || is_top_backfill_only)
            && (backfill_ids.is_empty() || is_top_backfill_only)
        {
            previous_id_set.difference(&new_id_set).cloned().collect()
        } else {
            HashSet::new()
        };
        if !removed_message_ids.is_empty() || !inserted_message_ids.is_empty() {
            let tags = |ids: &HashSet<String>| {
                let mut tags: Vec<String> = ids.iter().map(|id| RiseColdOpenTrace::hash_tag(id)).collect();
                tags.sort();
                tags.join(",")
            };
            RiseColdOpenTrace::mark(|| {
                format!(
                    "a3a1 list commit removed={} inserted={} rows={} prev={} removedTags={} insertedTags={}",
                    removed_message_ids.len(),
                    inserted_message_ids.len(),
                    new_ids.len(),
                    previous_id_set.len(),
                    tags(&removed_message_ids),
                    tags(&inserted_message_ids)
                )
            });
        }
        let width_changed_since_render = (self.manual_items_width - candidate.native_batch.width).abs() > 0.5;
        let height_changed_ids: HashSet<String> = if is_quiet_commit && !did_prepend && !width_changed_since_render {
            item_table
                .updated_ids
                .iter()
                .filter(|id| match (self.manual_items_by_id.get(*id), item_table.items_by_id.get(*id)) {
                    (Some(previous), Some(next)) => (previous.height - next.height).abs() > 0.5,
                    _ => false,
                })
                .cloned()
                .collect()
        } else {
            HashSet::new()
        };
        let reshaped_ids: HashSet<String> = if removed_message_ids.is_empty() {
            HashSet::new()
        } else {
            let previous_versions: HashMap<&str, i64> = previous_configuration
                .as_ref()
                .map(|previous| {
                    previous
                        .snapshot
                        .entries
                        .iter()
                        .filter(|entry| entry.message.is_some())
                        .map(|entry| (entry.id.as_str(), entry.content_version))
                        .collect()
                })
                .unwrap_or_default();
            new_configuration
                .snapshot
                .entries
                .iter()
                .filter(|entry| {
                    previous_versions
                        .get(entry.id.as_str())
                        .is_some_and(|version| *version != entry.content_version)
                })
                .map(|entry| entry.id.clone())
                .collect()
        };
        let mut update_animation = if scroll_safe_edge_commit.is_none()
            && (!reaction_update_ids.is_empty()
                || !inserted_message_ids.is_empty()
                || !removed_message_ids.is_empty()
                || !height_changed_ids.is_empty())
        {
            let dust_ids: HashSet<String> =
                removed_message_ids.iter().filter(|id| !id.starts_with("unread-")).cloned().collect();
            Some(RiseManualListTransactionUpdateAnimation::new(
                if inserted_message_ids.is_empty() { 0.34 } else { 0.4 },
                0.86,
                reaction_update_ids.union(&reshaped_ids).cloned().collect(),
                inserted_message_ids.clone(),
                removed_message_ids.clone(),
                dust_ids,
            ))
        } else {
            None
        };
        let resize_anchor = match self.viewport_resize_anchor.take() {
            Some(RiseManualListTransactionAnchor::Bottom)
                if width_changed_since_render
                    && !sent_own_message
                    && !new_configuration.can_load_newer
                    && !did_prepend =>
            {
                Some(RiseManualListTransactionAnchor::Bottom)
            }
            Some(RiseManualListTransactionAnchor::Item { id, viewport_offset })
                if width_changed_since_render && !sent_own_message && new_id_set.contains(&id) =>
            {
                Some(RiseManualListTransactionAnchor::Item { id, viewport_offset })
            }
            _ => None,
        };
        let anchor = if scroll_safe_edge_commit.is_some() {
            RiseManualListTransactionAnchor::PreserveVisible
        } else if reset_requested {
            self.initial_transaction_anchor(&new_configuration, &item_table, cx)
        } else if self.maintains_initial_bottom_pin && !self.is_user_scrolling(cx) {
            RiseManualListTransactionAnchor::Bottom
        } else if let Some(resize_anchor) = resize_anchor {
            resize_anchor
        } else if !new_configuration.can_load_newer
            && !travels_to_own_message
            && (sent_own_message || follows_incoming || (was_at_exact_bottom && !did_prepend))
        {
            RiseManualListTransactionAnchor::Bottom
        } else if let Some(removal_anchor) = self.removal_anchor(
            &removed_message_ids,
            previous_configuration.as_ref().map(|previous| previous.snapshot.as_ref()),
            &new_id_set,
            !new_configuration.can_load_newer,
            cx,
        ) {
            removal_anchor
        } else if let Some(stationary_anchor) = stationary_anchor.filter(|anchor| new_id_set.contains(&anchor.id)) {
            RiseManualListTransactionAnchor::Item {
                id: stationary_anchor.id,
                viewport_offset: stationary_anchor.viewport_offset,
            }
        } else {
            RiseManualListTransactionAnchor::PreserveVisible
        };
        if did_append || appended_message {
            let metrics = self.list_view.read(cx).scroll_metrics();
            tracing::warn!(
                target: "riseonly::X2173",
                chat = %new_configuration.chat_id,
                entries = new_configuration.snapshot.entries.len(),
                anchor = ?anchor,
                was_near_bottom,
                was_at_exact_bottom,
                follows_incoming,
                sent_own_message,
                new_last_is_own = ?new_last_is_own,
                can_load_newer = new_configuration.can_load_newer,
                bottom_pin = self.maintains_initial_bottom_pin,
                quiet = is_quiet_commit,
                inserted = inserted_message_ids.len(),
                offset = f32::from(metrics.content_offset_y),
                min_offset = f32::from(metrics.minimum_content_offset_y),
                max_offset = f32::from(metrics.maximum_content_offset_y),
                viewport_h = f32::from(metrics.viewport_size.height),
                content_h = f32::from(metrics.content_height),
                "commit anchor"
            );
        }
        if let Some(animation) = update_animation.as_mut()
            && !height_changed_ids.is_empty()
        {
            if anchor == RiseManualListTransactionAnchor::Bottom {
                animation.bottom_aligned_ids = height_changed_ids.clone();
            } else {
                animation.clips_growth_ids = height_changed_ids
                    .iter()
                    .filter(|id| match (self.manual_items_by_id.get(*id), item_table.items_by_id.get(*id)) {
                        (Some(previous), Some(next)) => next.height > previous.height + 0.5,
                        _ => false,
                    })
                    .cloned()
                    .collect();
            }
        }
        let transaction_scroll_request_id = self.pending_scroll_request.as_ref().map(|request| request.id);
        let parks_on_scroll_target =
            !reset_requested || self.scroll_request_target_is_prepared(&new_configuration, &item_table);
        let jump_animation = match self.pending_scroll_request.as_ref() {
            Some(request)
                if reset_requested
                    && self.initial_frame_ready_delivered
                    && request.animated
                    && RiseMessageScrollLandingPolicy::lands_in_commit(
                        &request.destination,
                        new_configuration.can_load_newer,
                    ) =>
            {
                Some(RiseManualListTransactionJumpAnimation::new(
                    self.jump_direction(request, &new_configuration, cx),
                    RiseManualListScrollAnimationPolicy::DURATION,
                ))
            }
            _ => None,
        };

        self.apply_generation = self.apply_generation.wrapping_add(1);
        let generation = self.apply_generation;
        self.is_applying_snapshot = true;
        if let Some(commit) = &scroll_safe_edge_commit {
            let mutation = commit.mutation;
            let changed_retained_count = commit
                .retained_ids
                .iter()
                .filter(|id| self.versions_by_id.get(*id) != new_configuration.snapshot.versions_by_id.get(*id))
                .count();
            MessagePaginationTrace::record(
                "render.commit.edge",
                &new_configuration.chat_id,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(new_configuration.snapshot.entries.len() as i64),
                None,
                Some(&format!(
                    "insertPrefix={} insertSuffix={} removePrefix={} removeSuffix={} changedRetained={changed_retained_count}",
                    mutation.inserted_prefix_count,
                    mutation.inserted_suffix_count,
                    mutation.removed_prefix_count,
                    mutation.removed_suffix_count,
                )),
                cx,
            );
        }
        let releases_composer_with_inserted_rows =
            !inserted_message_ids.is_empty() && self.composer_release_awaits_sent_row;
        let expected_width = candidate.native_batch.width;
        let allows_application_during_scroll = scroll_safe_edge_commit.is_some();
        let scroll_removable_ids = scroll_safe_edge_commit
            .map(|commit| commit.removable_visible_ids)
            .unwrap_or_default();
        let provider_items = item_table.items_by_id.clone();
        let committed = Rc::new(RiseMessageManualListViewCommittedTransaction {
            generation,
            new_configuration,
            committed_candidate: candidate,
            next_native_ids,
            items_by_id: item_table.items_by_id,
            transaction_scroll_request_id,
            reset_requested,
            parks_on_scroll_target,
            did_prepend,
            did_append,
            travels_to_own_message,
        });
        let weak = cx.weak_entity();
        let completion_weak = weak.clone();
        let discard_weak = weak;
        let mut transaction = RiseManualListTransaction::new(
            new_ids,
            anchor,
            Some(expected_width),
            Some(item_table.updated_ids),
            update_animation,
            jump_animation,
            edge_mutation,
            allows_application_during_scroll,
            scroll_removable_ids,
            Rc::new(move |id: &str, _width: f32, window: &mut Window, cx: &mut App| {
                provider_items
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| RiseMessageManualListView::missing_item_placeholder(id, "prepared", window, cx))
            }),
        );
        if releases_composer_with_inserted_rows
            && let Some(target) = self.take_held_composer_release_target(cx)
        {
            transaction.before_layout = Some(Rc::new(
                move |list_view: &mut RiseManualListView, window: &mut Window, cx: &mut Context<RiseManualListView>| {
                    list_view.finish_keyboard_viewport_transition(target.0, target.1, window, cx);
                },
            ));
        }
        self.list_view.update(cx, |list_view, cx| {
            list_view.enqueue(
                transaction,
                Some(Box::new(move |window: &mut Window, cx: &mut App| {
                    window.defer(cx, move |window, cx| {
                        let _ = completion_weak.update(cx, |this, cx| {
                            this.finish_committed_transaction(&committed, window, cx);
                        });
                    });
                })),
                Some(Box::new(move |window: &mut Window, cx: &mut App| {
                    window.defer(cx, move |window, cx| {
                        let _ = discard_weak.update(cx, |this, cx| {
                            this.discard_list_transaction(generation, window, cx);
                        });
                    });
                })),
                window,
                cx,
            );
        });
    }

    pub(in super::super) fn finish_committed_transaction(
        &mut self,
        committed: &RiseMessageManualListViewCommittedTransaction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.apply_generation != committed.generation {
            return;
        }
        let new_configuration = &committed.new_configuration;
        let committed_candidate = &committed.committed_candidate;
        let native_batch = &committed_candidate.native_batch;
        if !self.initial_frame_ready_delivered {
            let entries = new_configuration.snapshot.entries.len();
            RiseColdOpenTrace::mark(|| format!("list committed entries={entries}"));
        }

        self.deliver_deferred_mark_candidates_leaving_window(&new_configuration.snapshot, window, cx);
        self.entries_by_id = new_configuration.snapshot.entries_by_id.clone();
        self.versions_by_id = new_configuration.snapshot.versions_by_id.clone();
        self.native_layouts = native_batch.layouts.clone();
        self.hosted_text = native_batch.hosted_text.clone();
        self.native_layout_versions = native_batch.versions.clone();
        self.native_ownership_by_id = native_batch.ownership_by_id.clone();
        self.rendered_native_ids = committed.next_native_ids.clone();
        self.last_visible_predisplay_center = None;
        self.measured_heights = committed_candidate.fallback_heights.clone();
        self.measured_height_width = native_batch.width;
        self.manual_items_by_id = committed.items_by_id.clone();
        self.manual_items_width = native_batch.width;
        self.rendered_preparation_width = native_batch.width;
        self.rendered_configuration = Some(new_configuration.clone());
        self.rendered_configuration_revision = committed_candidate.configuration_revision;
        self.applied_source_version = Some(new_configuration.snapshot.source_version);
        self.is_applying_snapshot = false;
        tracing::warn!(
            target: "riseonly::X2173",
            chat = %new_configuration.chat_id,
            entries = new_configuration.snapshot.entries.len(),
            revision = committed_candidate.configuration_revision,
            "commit applied"
        );
        {
            let list_view = self.list_view.read(cx);
            let metrics = list_view.scroll_metrics();
            let last_id = new_configuration.snapshot.ids.last();
            tracing::warn!(
                target: "riseonly::X2173",
                chat = %new_configuration.chat_id,
                visible = list_view.visible_ids().len(),
                last_visible = last_id.is_some_and(|id| list_view.visible_ids().contains(id)),
                offset = f32::from(metrics.content_offset_y),
                max_offset = f32::from(metrics.maximum_content_offset_y),
                content_h = f32::from(metrics.content_height),
                "commit landed"
            );
        }
        self.clear_pending_presentation_commit(new_configuration, true);
        self.report_visible_mark_candidates_after_commit(window, cx);

        if native_batch.purpose == RiseMessageManualListViewNativePreparationPurpose::Foreground {
            self.rendered_foreground_preparation_range = native_batch.entry_range.clone();
        } else {
            self.complete_deferred_history_preparation(
                native_batch.entry_range.clone(),
                new_configuration,
                native_batch.width,
            );
        }

        let pending_request_id = self.pending_scroll_request.as_ref().map(|request| request.id);
        let consumed_scroll_reset = pending_request_id == committed.transaction_scroll_request_id;
        let lands_at_latest = self
            .pending_scroll_request
            .as_ref()
            .is_none_or(|request| request.destination != RiseMessageScrollDestination::Latest)
            || !new_configuration.can_load_newer;
        if committed.reset_requested {
            if consumed_scroll_reset && lands_at_latest && committed.parks_on_scroll_target {
                if let Some(request) = self.pending_scroll_request.clone() {
                    self.trigger_jump_highlight(&request, new_configuration, window, cx);
                }
                self.pending_scroll_reset = false;
                self.pending_scroll_request = None;
                if self.initial_frame_ready_delivered {
                    self.did_restore_initial_position = true;
                }
            }
        } else if self.initial_frame_ready_delivered {
            self.did_restore_initial_position = true;
        }
        if let Some(pending) = self
            .pending_history_edge_load
            .clone()
            .filter(|pending| pending.chat_id == new_configuration.chat_id)
            && RiseMessageHistoryEdgePolicy::commit_belongs_to_pending_request(
                pending.generation,
                committed_candidate.history_edge_load_generation,
            )
        {
            let observed_availability_progress =
                self.pending_history_edge_progress_generation == Some(pending.generation);
            match pending.edge {
                RiseMessageManualListViewHistoryEdge::Older => {
                    let did_complete = RiseMessageHistoryEdgePolicy::did_complete_pending_request(
                        committed.did_prepend,
                        observed_availability_progress,
                    );
                    if did_complete || !new_configuration.can_load_older {
                        self.clear_pending_history_edge_load();
                    }
                    if RiseMessageHistoryEdgePolicy::should_rearm_after_history_progress(
                        did_complete,
                        new_configuration.can_load_older,
                    ) {
                        self.load_older_armed = true;
                        self.last_load_older_at = 0.0;
                    }
                }
                RiseMessageManualListViewHistoryEdge::Newer => {
                    let did_complete = RiseMessageHistoryEdgePolicy::did_complete_pending_request(
                        committed.did_append,
                        observed_availability_progress,
                    );
                    if did_complete || !new_configuration.can_load_newer {
                        self.clear_pending_history_edge_load();
                    }
                    if RiseMessageHistoryEdgePolicy::should_rearm_after_history_progress(
                        did_complete,
                        new_configuration.can_load_newer,
                    ) {
                        self.load_newer_armed = true;
                        self.last_load_newer_at = 0.0;
                    }
                }
            }
        }
        self.history_edge_load_generation_by_source_version
            .remove(&new_configuration.snapshot.source_version);
        if committed.travels_to_own_message {
            self.list_view
                .update(cx, |list_view, cx| list_view.smart_scroll_to_bottom(true, window, cx));
        }
        let metrics = self.list_view.read(cx).scroll_metrics();
        self.scroll_metrics_did_change(&metrics, window, cx);
        self.schedule_initial_frame_readiness_if_needed(window, cx);
        if self.is_user_scrolling(cx)
            && let Some(pending_candidate) = self.pending_prepared_candidate.take()
        {
            self.finalize_prepared_snapshot(pending_candidate, window, cx);
            return;
        }
        if self.initial_frame_ready_delivered {
            self.begin_deferred_history_preparation_if_needed(window, cx);
        }
    }

    /// true when as many rows were removed as pulled in, fewer than a paging stride
    /// and every new id sits above the first surviving row
    pub(in super::super) fn is_top_backfill_only(
        backfill_ids: &HashSet<String>,
        new_ids: &[String],
        previous_id_set: &HashSet<String>,
        gone_count: usize,
        window_stride: usize,
    ) -> bool {
        if backfill_ids.is_empty() || backfill_ids.len() != gone_count || gone_count >= window_stride {
            return false;
        }
        let mut seen = 0;
        for id in new_ids {
            if previous_id_set.contains(id) {
                return seen == backfill_ids.len();
            }
            if !backfill_ids.contains(id) {
                return false;
            }
            seen += 1;
        }
        false
    }

    pub(in super::super) fn reaction_update_ids(
        previous: Option<&RiseMessageCollectionConfiguration>,
        next: &RiseMessageCollectionConfiguration,
    ) -> HashSet<String> {
        let Some(previous) = previous else {
            return HashSet::new();
        };
        if previous.chat_id != next.chat_id || previous.snapshot.ids != next.snapshot.ids {
            return HashSet::new();
        }

        let mut previous_messages: HashMap<&str, &Arc<GetMessageMessage>> =
            HashMap::with_capacity(previous.snapshot.entries.len());
        for entry in &previous.snapshot.entries {
            if let Some(message) = entry.message.as_ref() {
                previous_messages.entry(entry.id.as_str()).or_insert(message);
            }
        }
        next.snapshot
            .entries
            .iter()
            .filter_map(|entry| {
                let message = entry.message.as_ref()?;
                let previous_message = previous_messages.get(entry.id.as_str())?;
                (previous_message.reactions != message.reactions
                    || previous_message.sparks_total != message.sparks_total
                    || previous_message.my_sparks != message.my_sparks)
                    .then(|| entry.id.clone())
            })
            .collect()
    }

    pub fn stop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_animated_media_playback_active(false, window, cx);
        if self.was_user_scrolling {
            let configuration = self.rendered_configuration.clone().or_else(|| self.configuration.clone());
            self.deliver_scroll_activity_changed(false, configuration, window, cx);
            self.was_user_scrolling = false;
        }
        self.persist_scroll_position(None, cx);
        ChatScrollPositionStore::flush_pending_to_disk(cx);
        self.initial_frame_ready_delivered = false;
        self.has_observed_history_scroll_interaction = false;
        self.pending_reveal_highlight = None;
        self.pending_presentation_commit = None;
        self.presentation_chat_id = None;
        self.presentation_epoch = None;
        self.initial_frame_owner = None;
        self.maintains_initial_bottom_pin = false;
        self.viewport_resize_anchor = None;
        self.queued_pre_reveal_configuration = None;
        self.pending_initial_frame_reveal = None;
        self.cancel_deferred_history_preparation();
        self.render_preparation_generation = self.render_preparation_generation.wrapping_add(1);
        self.cancel_render_preparation_task();
        self.pending_render_key = None;
        self.pending_render_completion = None;
        self.initial_viewport_readiness_generation = self.initial_viewport_readiness_generation.wrapping_add(1);
        self.initial_viewport_readiness_pass_in_flight = false;
        self.initial_viewport_readiness_watchdog = None;
        self.apply_generation = self.apply_generation.wrapping_add(1);
        self.keyboard_target_is_visible = false;
        self.list_view.update(cx, |list_view, cx| list_view.stop(window, cx));
        self.is_applying_snapshot = false;
        self.native_preparation_generation = self.native_preparation_generation.wrapping_add(1);
        self.native_preparation_task = None;
        self.pending_native_configuration = None;
        self.pending_native_completion = None;
        self.pending_native_entry_range = None;
        self.pending_native_purpose = RiseMessageManualListViewNativePreparationPurpose::Foreground;
        self.prepared_snapshot_finalization_generation = self.prepared_snapshot_finalization_generation.wrapping_add(1);
        self.prepared_snapshot_finalization_task = None;
        self.candidate_being_predisplayed = None;
        self.visible_predisplay_task = None;
        self.cancel_directional_media_prefetch(cx);
        self.pending_prepared_candidate = None;
        self.history_edge_load_generation = self.history_edge_load_generation.wrapping_add(1);
        self.clear_pending_history_edge_load();
        self.history_edge_load_generation_by_source_version.clear();
        self.pending_runtime_height_corrections.clear();
        self.runtime_height_correction_flush_scheduled = false;
        self.last_reported_mark_candidate_ids.clear();
        self.deferred_mark_candidates.clear();
        self.deferred_mark_chat_id = None;
        self.reset_viewport_state_delivery();
        self.pending_scroll_request = None;
    }

    pub(in super::super) fn enqueue_prepared_candidate(
        &mut self,
        candidate: RiseMessageManualListViewPreparedSnapshotCandidate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.enqueue_prepared_snapshot(
            candidate.configuration,
            candidate.native_batch,
            candidate.fallback_heights,
            Some(candidate.configuration_revision),
            candidate.history_edge_load_generation,
            window,
            cx,
        );
    }

    pub(in super::super) fn adopt_latest_configuration(&self, candidate: &mut RiseMessageManualListViewPreparedSnapshotCandidate) {
        if let Some(latest_configuration) = self.configuration.as_ref()
            && latest_configuration.chat_id == candidate.configuration.chat_id
            && latest_configuration.snapshot.source_version == candidate.configuration.snapshot.source_version
        {
            candidate.configuration = latest_configuration.clone();
        }
    }

    pub(in super::super) fn finalize_prepared_snapshot(
        &mut self,
        initial_candidate: RiseMessageManualListViewPreparedSnapshotCandidate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prepared_snapshot_finalization_generation = self.prepared_snapshot_finalization_generation.wrapping_add(1);
        let generation = self.prepared_snapshot_finalization_generation;
        self.prepared_snapshot_finalization_task = None;
        let measurement_range = self.fallback_finalization_range(&initial_candidate);
        let traces_cold_open = !self.initial_frame_ready_delivered;
        self.prepared_snapshot_finalization_task = Some(cx.spawn_in(window, async move |this, cx| {
            let trace_start = RiseClock::monotonic();
            let trace_ms = move || RiseClock::monotonic().saturating_sub(trace_start).as_millis();
            if traces_cold_open {
                let range = measurement_range.len();
                let fallback = initial_candidate.fallback_heights.len();
                RiseColdOpenTrace::mark(|| format!("list finalize begin range={range} fallback={fallback}"));
            }
            let measurement =
                Self::measure_fallback_heights(this.clone(), initial_candidate, measurement_range, cx).await;
            if traces_cold_open {
                let cancelled = matches!(measurement, RiseMessageManualListViewFallbackMeasurementResult::Cancelled);
                let current = this
                    .read_with(cx, |this, _| this.prepared_snapshot_finalization_generation == generation)
                    .unwrap_or(false);
                RiseColdOpenTrace::mark(|| {
                    format!("list finalize measured ms={} cancelled={cancelled} current={current}", trace_ms())
                });
            }
            let measured_candidate = match measurement {
                RiseMessageManualListViewFallbackMeasurementResult::Cancelled => {
                    let _ = this.update_in(cx, |this, _, _| {
                        if this.prepared_snapshot_finalization_generation == generation {
                            this.prepared_snapshot_finalization_task = None;
                        }
                    });
                    return;
                }
                RiseMessageManualListViewFallbackMeasurementResult::Paused(candidate) => {
                    let _ = this.update_in(cx, |this, window, cx| {
                        if this.prepared_snapshot_finalization_generation != generation {
                            return;
                        }
                        this.prepared_snapshot_finalization_task = None;
                        let scroll_safe_edge_commit = if this.is_applying_snapshot {
                            None
                        } else {
                            this.scroll_safe_edge_commit(&candidate, true, cx)
                        };
                        if let Some(scroll_safe_edge_commit) = scroll_safe_edge_commit {
                            this.pending_prepared_candidate = None;
                            this.commit_prepared_snapshot(candidate, Some(scroll_safe_edge_commit), window, cx);
                        } else {
                            this.trace_deferred_render(&candidate, "fallback-measurement-paused", cx);
                            this.set_pending_prepared_candidate(Some(candidate), window, cx);
                        }
                    });
                    return;
                }
                RiseMessageManualListViewFallbackMeasurementResult::Completed(candidate) => candidate,
            };
            let mut measured_candidate = Some(measured_candidate);
            let Ok(Some((inputs, lottie_urls, bitmap_urls, initial_frame_ready))) =
                this.update_in(cx, |this, window, cx| {
                    if this.prepared_snapshot_finalization_generation != generation {
                        return None;
                    }
                    let mut candidate = measured_candidate.take()?;
                    if candidate.configuration_revision != this.configuration_revision {
                        this.prepared_snapshot_finalization_task = None;
                        this.enqueue_prepared_candidate(candidate, window, cx);
                        return None;
                    }
                    this.adopt_latest_configuration(&mut candidate);
                    let inputs = if candidate.native_batch.purpose
                        == RiseMessageManualListViewNativePreparationPurpose::Foreground
                    {
                        let inputs = this.predisplay_inputs(&candidate, window, cx);
                        this.candidate_being_predisplayed = Some(candidate.clone());
                        inputs
                    } else {
                        this.deferred_predisplay_inputs(&candidate, cx)
                    };
                    let lottie_urls = this.lottie_predisplay_urls(&candidate, window, cx);
                    let bitmap_urls = this.bitmap_predisplay_urls(&candidate, window, cx);
                    let initial_frame_ready = this.initial_frame_ready_delivered;
                    measured_candidate = Some(candidate);
                    Some((inputs, lottie_urls, bitmap_urls, initial_frame_ready))
                })
            else {
                return;
            };
            let executor = cx.background_executor().clone();
            let input_count = inputs.len();
            let requests = executor.spawn(async move { Self::make_native_predisplay_requests(inputs) }).await;
            if traces_cold_open {
                let current = this
                    .read_with(cx, |this, _| this.prepared_snapshot_finalization_generation == generation)
                    .unwrap_or(false);
                RiseColdOpenTrace::mark(|| {
                    format!("list finalize requests ms={} inputs={input_count} current={current}", trace_ms())
                });
            }
            let text_predisplay = AsyncTextBubblePredisplayCache::shared().prewarm(requests, &executor);
            if initial_frame_ready {
                text_predisplay.await;
            } else {
                let maximum_wait = Duration::from_nanos(160_000_000);
                let async_app: &AsyncApp = cx;
                let media_predisplay = async {
                    futures::join!(
                        LottieStickerPrefetch::prepare_first_frames(&lottie_urls, maximum_wait, async_app),
                        CleverImageDiskPrefetch::prepare_first_images(&bitmap_urls, maximum_wait, async_app),
                    );
                    if traces_cold_open {
                        let (lottie, bitmaps) = (lottie_urls.len(), bitmap_urls.len());
                        RiseColdOpenTrace::mark(|| {
                            format!("list finalize media ms={} lottie={lottie} bitmaps={bitmaps}", trace_ms())
                        });
                    }
                };
                futures::join!(text_predisplay, media_predisplay);
            }
            if traces_cold_open {
                let (current, source, revision) = this
                    .read_with(cx, |this, _| match measured_candidate.as_ref() {
                        Some(candidate) => (
                            this.prepared_snapshot_finalization_generation == generation,
                            this.requested_source_version == Some(candidate.configuration.snapshot.source_version),
                            candidate.configuration_revision == this.configuration_revision,
                        ),
                        None => (this.prepared_snapshot_finalization_generation == generation, false, false),
                    })
                    .unwrap_or((false, false, false));
                RiseColdOpenTrace::mark(|| {
                    format!(
                        "list finalize text ms={} cancelled=false current={current} source={source} revision={revision}",
                        trace_ms()
                    )
                });
            }
            let _ = this.update_in(cx, |this, window, cx| {
                let Some(mut candidate) = measured_candidate.take() else {
                    return;
                };
                if this.prepared_snapshot_finalization_generation != generation
                    || this.requested_source_version != Some(candidate.configuration.snapshot.source_version)
                    || this.configuration.as_ref().map(|configuration| configuration.chat_id.as_str())
                        != Some(candidate.configuration.chat_id.as_str())
                {
                    tracing::warn!(
                        target: "riseonly::X1793",
                        current = this.prepared_snapshot_finalization_generation == generation,
                        requested = ?this.requested_source_version,
                        candidate = candidate.configuration.snapshot.source_version,
                        "finalize dropped"
                    );
                    return;
                }
                this.prepared_snapshot_finalization_task = None;
                this.candidate_being_predisplayed = None;

                if candidate.configuration_revision != this.configuration_revision {
                    this.enqueue_prepared_candidate(candidate, window, cx);
                    return;
                }
                this.adopt_latest_configuration(&mut candidate);
                if this.is_user_scrolling(cx) {
                    let scroll_safe_edge_commit = if this.is_applying_snapshot {
                        None
                    } else {
                        this.scroll_safe_edge_commit(&candidate, true, cx)
                    };
                    let Some(scroll_safe_edge_commit) = scroll_safe_edge_commit else {
                        this.trace_deferred_render(&candidate, "post-predisplay-edge-not-safe", cx);
                        this.set_pending_prepared_candidate(Some(candidate), window, cx);
                        return;
                    };
                    this.commit_prepared_snapshot(candidate, Some(scroll_safe_edge_commit), window, cx);
                    return;
                }
                if this.is_applying_snapshot {
                    this.set_pending_prepared_candidate(Some(candidate), window, cx);
                    return;
                }
                this.commit_prepared_snapshot(candidate, None, window, cx);
            });
        }));
    }

    pub(in super::super) fn unresolved_fallback_geometry_count_at_indices(
        &self,
        candidate: &RiseMessageManualListViewPreparedSnapshotCandidate,
        indices: &[usize],
        reusing_retained_item_ids: &HashSet<String>,
    ) -> i64 {
        let entries = &candidate.configuration.snapshot.entries;
        indices
            .iter()
            .filter(|&&index| {
                index < entries.len()
                    && self.fallback_geometry_is_unresolved(candidate, index, reusing_retained_item_ids)
            })
            .count() as i64
    }

    pub(in super::super) fn is_near_bottom(&self, cx: &App) -> bool {
        if self
            .rendered_configuration
            .as_ref()
            .or(self.configuration.as_ref())
            .is_some_and(|configuration| configuration.can_load_newer)
        {
            return false;
        }
        self.distance_from_bottom(cx) <= ChatScrollPositionStore::NEAR_BOTTOM_THRESHOLD
    }
}
