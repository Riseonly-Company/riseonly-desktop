use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;
use gpui::{App, Context, Pixels, Window};
use rise_core::foundation::foundation_date::{RiseClock, TimeInterval};
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::modules::chat::components::chat::async_render::chat_message_render_snapshot::{ChatMessageRenderEntryKind, ChatMessageRenderSnapshot};
use crate::modules::chat::components::chat::async_render::chat_performance_trace::ChatPerformanceTrace;
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_types::RiseManualListScrollMetrics;
use crate::modules::user::engine::rise_meaningful_activity::RiseMeaningfulActivity;
use crate::modules::chat::engine::messages::rise_message_unread_marks::{RiseUnreadMarkCandidate, RiseUnreadMarkViewport};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::modules::chat::stores::message::message_services::message_pagination_trace::MessagePaginationTrace;
use super::super::rise_message_list_view::*;

impl RiseMessageManualListView {
    pub(in super::super) fn set_pending_prepared_candidate(
        &mut self,
        candidate: Option<RiseMessageManualListViewPreparedSnapshotCandidate>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pending_prepared_candidate = candidate;
        if self.pending_prepared_candidate.is_some() {
            self.schedule_parked_candidate_retry_if_needed(window, cx);
        }
    }

    pub(in super::super) fn publish_viewport_state(
        &mut self,
        metrics: &RiseManualListScrollMetrics,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.initial_frame_ready_delivered || !self.did_restore_initial_position || self.is_applying_snapshot {
            return;
        }
        let Some(configuration) = self.rendered_configuration.as_ref().or(self.configuration.as_ref()) else {
            return;
        };
        let next = RiseMessageViewportPolicy::resolve(
            if configuration.snapshot.entries.is_empty() { 0.0 } else { f32::from(metrics.distance_from_bottom) },
            configuration.can_load_newer,
            self.pending_viewport_state.or(self.published_viewport_state),
        );
        if Some(next) == self.pending_viewport_state {
            return;
        }
        if self.pending_viewport_state.is_none() && Some(next) == self.published_viewport_state {
            return;
        }

        self.pending_viewport_state = Some(next);
        if self.viewport_state_delivery_scheduled {
            return;
        }
        self.viewport_state_delivery_scheduled = true;
        let generation = self.viewport_state_delivery_generation;
        cx.defer_in(window, move |this, window, cx| {
            this.viewport_state_delivery_scheduled = false;
            if this.viewport_state_delivery_generation != generation {
                return;
            }
            let Some(state) = this.pending_viewport_state.take() else {
                return;
            };
            if Some(state) == this.published_viewport_state {
                return;
            }
            this.published_viewport_state = Some(state);
            let Some(configuration) = this.rendered_configuration.clone().or_else(|| this.configuration.clone()) else {
                return;
            };
            let metrics = this.list_view.read(cx).scroll_metrics();
            let distance_bottom = f32::from(metrics.distance_from_bottom).round() as i64;
            let detail = format!(
                "distanceBottom={distance_bottom} canLoadNewer={} atLatest={} showsJump={}",
                configuration.can_load_newer, state.is_at_latest, state.shows_jump_to_latest,
            );
            MessagePaginationTrace::record(
                "viewport.state",
                &configuration.chat_id,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(configuration.snapshot.entries.len() as i64),
                None,
                Some(&detail),
                cx,
            );
            ChatPerformanceTrace::shared().record(
                "viewport.state",
                HashMap::from([
                    ("distance_bottom".to_owned(), distance_bottom.to_string()),
                    ("is_at_latest".to_owned(), if state.is_at_latest { "1" } else { "0" }.to_owned()),
                    ("shows_jump".to_owned(), if state.shows_jump_to_latest { "1" } else { "0" }.to_owned()),
                ]),
            );
            if let Some(callback) = configuration.on_viewport_state_changed.as_ref() {
                callback(&state, window, cx);
            }
        });
    }

    pub(in super::super) fn reset_viewport_state_delivery(&mut self) {
        self.viewport_state_delivery_generation = self.viewport_state_delivery_generation.wrapping_add(1);
        self.viewport_state_delivery_scheduled = false;
        self.pending_viewport_state = None;
        self.published_viewport_state = None;
    }

    pub(in super::super) fn active_scroll_metrics_did_change(
        &mut self,
        metrics: &RiseManualListScrollMetrics,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.note_mark_scroll_movement(metrics.content_offset_y);
        self.last_active_scroll_metrics_handled = Some(*metrics);
        self.update_animated_media_playback_for_scroll(metrics, cx);
        if !(metrics.is_tracking || metrics.is_dragging || metrics.is_decelerating)
            || !self.did_restore_initial_position
        {
            return;
        }
        let Some(configuration) = self.rendered_configuration.clone() else {
            return;
        };
        if configuration.snapshot.entries.is_empty() {
            return;
        }
        self.has_observed_history_scroll_interaction = true;
        self.evaluate_history_edge_prefetch(metrics, &configuration, window, cx);
    }

    pub(in super::super) fn scroll_metrics_did_change(
        &mut self,
        metrics: &RiseManualListScrollMetrics,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let already_handled_this_frame = self.last_active_scroll_metrics_handled.as_ref() == Some(metrics);
        if !already_handled_this_frame {
            self.update_animated_media_playback_for_scroll(metrics, cx);
        }
        let now_scrolling = metrics.is_tracking || metrics.is_dragging || metrics.is_decelerating;
        let did_finish_user_scrolling = !now_scrolling && self.was_user_scrolling;
        if now_scrolling {
            self.has_observed_history_scroll_interaction = true;
        }
        if now_scrolling != self.was_user_scrolling {
            let configuration = self.rendered_configuration.clone().or_else(|| self.configuration.clone());
            self.deliver_scroll_activity_changed(now_scrolling, configuration, window, cx);
        }
        if now_scrolling && !self.was_user_scrolling {
            self.maintains_initial_bottom_pin = false;
            self.viewport_resize_anchor = None;
            self.cancel_active_deferred_history_work_for_scroll();
            self.pause_prepared_snapshot_finalization_for_scroll(window, cx);
            self.schedule_parked_candidate_retry_if_needed(window, cx);
        }
        self.was_user_scrolling = now_scrolling;
        if did_finish_user_scrolling {
            self.persist_scroll_position(None, cx);
        } else if !now_scrolling && self.is_at_exact_bottom(cx) {
            let rendered_configuration = self.rendered_configuration.clone();
            if rendered_configuration.is_some_and(|rendered| rendered.saved_scroll_position(cx).is_some()) {
                self.persist_scroll_position(None, cx);
            }
        }

        if !now_scrolling && !self.initial_frame_ready_delivered {
            self.schedule_initial_frame_readiness_if_needed(window, cx);
        }

        self.publish_viewport_state(metrics, window, cx);

        let configuration = self
            .rendered_configuration
            .clone()
            .filter(|configuration| self.did_restore_initial_position && !configuration.snapshot.entries.is_empty());
        let Some(configuration) = configuration else {
            if !now_scrolling {
                self.flush_deferred_work_if_possible(window, cx);
            }
            return;
        };
        if !already_handled_this_frame && !HoldContextMenuPresenter::has_active_request(cx) {
            self.evaluate_history_edge_prefetch(metrics, &configuration, window, cx);
        }
        if !now_scrolling {
            self.flush_deferred_work_if_possible(window, cx);
        }
    }

    pub(in super::super) fn deliver_scroll_activity_changed(
        &self,
        is_scrolling: bool,
        configuration: Option<Rc<RiseMessageCollectionConfiguration>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(callback) = configuration.and_then(|configuration| configuration.on_scroll_activity_changed.clone())
        else {
            return;
        };
        cx.defer_in(window, move |_, window, cx| callback(is_scrolling, window, cx));
    }

    pub(in super::super) fn pause_prepared_snapshot_finalization_for_scroll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.prepared_snapshot_finalization_task.is_none() {
            return;
        }
        let Some(candidate_being_predisplayed) = self.candidate_being_predisplayed.take() else {
            return;
        };
        self.prepared_snapshot_finalization_generation = self.prepared_snapshot_finalization_generation.wrapping_add(1);
        self.prepared_snapshot_finalization_task = None;
        let scroll_safe_edge_commit = if self.is_applying_snapshot {
            None
        } else {
            self.scroll_safe_edge_commit(&candidate_being_predisplayed, true, cx)
        };
        if let Some(scroll_safe_edge_commit) = scroll_safe_edge_commit {
            self.set_pending_prepared_candidate(None, window, cx);
            self.commit_prepared_snapshot(candidate_being_predisplayed, Some(scroll_safe_edge_commit), window, cx);
        } else {
            self.trace_deferred_render(&candidate_being_predisplayed, "predisplay-paused", cx);
            self.set_pending_prepared_candidate(Some(candidate_being_predisplayed), window, cx);
        }
    }

    pub(in super::super) fn readable_ids_did_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.initial_frame_ready_delivered {
            return;
        }
        let readable = self.list_view.read(cx).readable_ids().to_vec();
        self.report_visible_mark_candidates(&readable, window, cx);
    }

    pub(in super::super) fn visible_ids_did_change(&mut self, ids: &[String], window: &mut Window, cx: &mut Context<Self>) {
        if !self.initial_frame_ready_delivered {
            return;
        }
        if ids.is_empty() {
            self.last_reported_mark_candidate_ids.clear();
            self.cancel_directional_media_prefetch(cx);
            return;
        }
        self.update_video_note_floating(ids, cx);
        let metrics = self.list_view.read(cx).scroll_metrics();
        if (metrics.is_tracking || metrics.is_dragging || metrics.is_decelerating)
            && let Some(configuration) = self.rendered_configuration.clone()
        {
            self.evaluate_history_edge_prefetch(&metrics, &configuration, window, cx);
        }
        self.retry_pending_prepared_candidate_during_scroll(window, cx);
        self.update_deferred_history_viewport(ids);
        self.resume_deferred_history_preparation_if_possible(window, cx);
        self.prefetch_directional_media(ids, cx);
        if ids.iter().any(|id| self.rendered_native_ids.contains(id)) {
            self.prewarm_native_text(ids, window, cx);
        }
        self.report_visible_mark_candidates(ids, window, cx);
    }

    /// a stationary viewport sends no visible range change after a commit so rows the commit
    /// published under the old config get their turn here
    pub(in super::super) fn report_visible_mark_candidates_after_commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.initial_frame_ready_delivered {
            return;
        }
        let ids = self.visible_ids(cx);
        if ids.is_empty() {
            return;
        }
        if let Some(configuration) = self.rendered_configuration.as_ref() {
            let flagged: Vec<String> = ids
                .iter()
                .filter(|id| {
                    configuration.snapshot.entries_by_id.get(*id).and_then(|entry| entry.message.as_ref()).is_some_and(
                        |message| message.has_unread_reactions || message.is_mentioned_you || message.is_replied_you,
                    )
                })
                .cloned()
                .collect();
            if !flagged.is_empty() {
                tracing::info!(target: "riseonly::X796", "commit re-reports flagged rows={}", flagged.len());
            }
            for id in &flagged {
                self.last_reported_mark_candidate_ids.remove(id);
            }
        }
        self.report_visible_mark_candidates(&ids, window, cx);
    }

    /// reports each row once per stay and supplies a live reader
    /// so reactions arriving on a still viewport go through the same ledger
    pub(in super::super) fn report_visible_mark_candidates(&mut self, ids: &[String], window: &mut Window, cx: &mut Context<Self>) {
        self.update_meaningful_activity(cx);
        let Some(configuration) = self.rendered_configuration.clone() else {
            self.last_reported_mark_candidate_ids.clear();
            self.deferred_mark_candidates.clear();
            return;
        };
        let Some(callback) = configuration.on_visible_mark_candidates_changed.clone() else {
            self.last_reported_mark_candidate_ids.clear();
            self.deferred_mark_candidates.clear();
            return;
        };
        if !self.can_report_unread_mark_visibility() {
            self.last_reported_mark_candidate_ids.clear();
            return;
        }
        let visible: HashSet<String> = {
            let list_view = self.list_view.read(cx);
            let readable: HashSet<&String> = list_view.readable_ids().iter().collect();
            ids.iter().filter(|id| readable.contains(id)).cloned().collect()
        };
        let fresh: Vec<String> =
            visible.iter().filter(|id| !self.last_reported_mark_candidate_ids.contains(*id)).cloned().collect();
        self.last_reported_mark_candidate_ids = visible
            .into_iter()
            .filter(|id| configuration.snapshot.entries_by_id.contains_key(id))
            .collect();
        if fresh.is_empty() {
            return;
        }
        let mut candidates: HashMap<String, RiseMessageManualListViewDeferredMarkCandidate> =
            HashMap::with_capacity(fresh.len());
        for id in fresh {
            let Some(entry) = configuration.snapshot.entries_by_id.get(&id) else {
                continue;
            };
            if entry.kind != ChatMessageRenderEntryKind::Message {
                continue;
            }
            let Some(message) = entry.message.as_ref() else {
                continue;
            };
            let server_id = message.sai_server_id();
            if message.is_temp || server_id.is_empty() {
                continue;
            }
            let previous = candidates.get(&server_id).map(|value| &value.candidate);
            let candidate = RiseUnreadMarkCandidate {
                message_id: server_id.clone(),
                is_mentioned_you: message.is_mentioned_you || previous.is_some_and(|previous| previous.is_mentioned_you),
                has_unread_reactions: message.has_unread_reactions
                    || previous.is_some_and(|previous| previous.has_unread_reactions),
                is_replied_you: message.is_replied_you || previous.is_some_and(|previous| previous.is_replied_you),
            };
            candidates.insert(server_id, RiseMessageManualListViewDeferredMarkCandidate { entry_id: id, candidate });
        }
        if candidates.is_empty() {
            return;
        }
        if self.is_user_scrolling(cx) {
            self.deliver_mark_viewport(Vec::new(), &configuration, callback, window, cx);
            if self.deferred_mark_chat_id.as_deref() != Some(configuration.chat_id.as_str()) {
                self.deferred_mark_candidates.clear();
                self.deferred_mark_chat_id = Some(configuration.chat_id.clone());
            }
            for (server_id, next) in candidates {
                let merged = match self.deferred_mark_candidates.remove(&server_id) {
                    Some(previous) => RiseMessageManualListViewDeferredMarkCandidate {
                        entry_id: next.entry_id,
                        candidate: RiseUnreadMarkCandidate {
                            message_id: next.candidate.message_id,
                            is_mentioned_you: previous.candidate.is_mentioned_you || next.candidate.is_mentioned_you,
                            has_unread_reactions: previous.candidate.has_unread_reactions
                                || next.candidate.has_unread_reactions,
                            is_replied_you: previous.candidate.is_replied_you || next.candidate.is_replied_you,
                        },
                    },
                    None => next,
                };
                self.deferred_mark_candidates.insert(server_id, merged);
            }
            self.arm_deferred_mark_hold_still(Self::DEFERRED_MARK_HOLD_STILL_DELAY, window, cx);
            return;
        }
        let candidates = candidates.into_values().map(|value| value.candidate).collect();
        self.deliver_mark_viewport(candidates, &configuration, callback, window, cx);
    }

    pub(in super::super) fn deliver_mark_viewport(
        &self,
        candidates: Vec<RiseUnreadMarkCandidate>,
        configuration: &RiseMessageCollectionConfiguration,
        callback: RiseMessageListViewMarkCandidatesHandler,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let chat_id = configuration.chat_id.clone();
        let epoch = configuration.presentation_epoch;
        let position_key = configuration.scroll_position_key.clone();
        let account_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let is_current = {
            let chat_id = chat_id.clone();
            let position_key = position_key.clone();
            let account_id = account_id.clone();
            let account_epoch = account_epoch.clone();
            move |this: &RiseMessageManualListView| -> bool {
                this.rendered_configuration.as_ref().is_some_and(|rendered| {
                    rendered.chat_id == chat_id
                        && rendered.presentation_epoch == epoch
                        && rendered.scroll_position_key == position_key
                }) && SaiAccountCacheScope::active_user_id() == account_id
                    && SaiAccountCacheScope::active_epoch() == account_epoch
            }
        };
        let reader_is_current = is_current.clone();
        let reader = cx.weak_entity();
        let viewport = RiseUnreadMarkViewport::new(candidates.clone(), move |cx: &mut App| {
            let Some(this) = reader.upgrade() else {
                return Vec::new();
            };
            let this = this.read(cx);
            if !reader_is_current(this) {
                return Vec::new();
            }
            this.current_unread_mark_candidates(cx)
        });
        cx.defer_in(window, move |this, window, cx| {
            if this.presentation_chat_id.as_deref() != Some(chat_id.as_str())
                || this.presentation_epoch != epoch
                || !is_current(this)
            {
                return;
            }
            if candidates.is_empty() && this.current_unread_mark_candidates(cx).is_empty() {
                return;
            }
            callback(viewport, window, cx);
        });
    }

    pub(in super::super) fn can_report_unread_mark_visibility(&self) -> bool {
        self.initial_frame_ready_delivered
            && self.requested_animated_media_playback_active
            && self.is_in_window
            && self.window_is_active
            && self.list_alpha > 0.01
    }

    pub(in super::super) fn update_meaningful_activity(&mut self, cx: &mut Context<Self>) {
        let qualifies = self.can_report_unread_mark_visibility() && !self.current_unread_mark_candidates(cx).is_empty();
        let source = self.meaningful_activity_source;
        let weak = cx.weak_entity();
        RiseMeaningfulActivity::shared(cx).update(cx, |activity, cx| {
            activity.set_source(
                source,
                qualifies,
                move |cx: &App| {
                    let Some(this) = weak.upgrade() else {
                        return false;
                    };
                    let this = this.read(cx);
                    this.can_report_unread_mark_visibility() && !this.current_unread_mark_candidates(cx).is_empty()
                },
                cx,
            );
        });
    }

    pub(in super::super) fn current_unread_mark_candidates(&self, cx: &App) -> Vec<RiseUnreadMarkCandidate> {
        if !self.can_report_unread_mark_visibility() {
            return Vec::new();
        }
        let Some(configuration) = self.rendered_configuration.as_ref() else {
            return Vec::new();
        };
        self.list_view
            .read(cx)
            .readable_ids()
            .iter()
            .filter_map(|id| {
                let entry = configuration.snapshot.entries_by_id.get(id)?;
                if entry.kind != ChatMessageRenderEntryKind::Message {
                    return None;
                }
                let message = entry.message.as_ref()?;
                let message_id = message.sai_server_id();
                if message.is_temp || message_id.is_empty() {
                    return None;
                }
                Some(RiseUnreadMarkCandidate {
                    message_id,
                    is_mentioned_you: message.is_mentioned_you,
                    has_unread_reactions: message.has_unread_reactions,
                    is_replied_you: message.is_replied_you,
                })
            })
            .collect()
    }

    pub(in super::super) fn deliver_deferred_mark_candidates_leaving_window(
        &mut self,
        snapshot: &ChatMessageRenderSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let leaving_ids: Vec<String> = self
            .deferred_mark_candidates
            .iter()
            .filter(|(_, value)| !snapshot.entries_by_id.contains_key(&value.entry_id))
            .map(|(id, _)| id.clone())
            .collect();
        if leaving_ids.is_empty() {
            return;
        }
        let candidates: Vec<RiseUnreadMarkCandidate> = leaving_ids
            .iter()
            .filter_map(|id| self.deferred_mark_candidates.remove(id).map(|value| value.candidate))
            .collect();
        let Some(configuration) = self.rendered_configuration.clone() else {
            return;
        };
        if self.deferred_mark_chat_id.as_deref() != Some(configuration.chat_id.as_str()) {
            return;
        }
        let Some(callback) = configuration.on_visible_mark_candidates_changed.clone() else {
            return;
        };
        self.deliver_mark_viewport(candidates, &configuration, callback, window, cx);
    }

    pub(in super::super) fn flush_deferred_mark_candidates_if_idle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_user_scrolling(cx) {
            return;
        }
        self.flush_deferred_mark_candidates(window, cx);
    }

    pub(in super::super) const DEFERRED_MARK_HOLD_STILL_DELAY: TimeInterval = 2.6;

    pub(in super::super) fn note_mark_scroll_movement(&mut self, offset_y: Pixels) {
        if Some(offset_y) == self.last_mark_scroll_offset_y {
            return;
        }
        self.last_mark_scroll_offset_y = Some(offset_y);
        self.last_mark_scroll_movement_at = RiseClock::monotonic().as_secs_f64();
    }

    /// one pending check at a time, it re-arms for the remainder while the offset keeps moving
    pub(in super::super) fn arm_deferred_mark_hold_still(&mut self, delay: TimeInterval, window: &mut Window, cx: &mut Context<Self>) {
        if self.deferred_mark_hold_still_work.is_some() {
            return;
        }
        self.deferred_mark_hold_still_work = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs_f64(delay.max(0.0))).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.deferred_mark_hold_still_work = None;
                if this.deferred_mark_candidates.is_empty() {
                    return;
                }
                let still = RiseClock::monotonic().as_secs_f64() - this.last_mark_scroll_movement_at;
                if still >= Self::DEFERRED_MARK_HOLD_STILL_DELAY {
                    this.flush_deferred_mark_candidates(window, cx);
                } else {
                    this.arm_deferred_mark_hold_still(Self::DEFERRED_MARK_HOLD_STILL_DELAY - still, window, cx);
                }
            });
        }));
    }

    pub(in super::super) fn flush_deferred_mark_candidates(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.deferred_mark_hold_still_work = None;
        if self.deferred_mark_candidates.is_empty() {
            return;
        }
        let candidates: Vec<RiseUnreadMarkCandidate> =
            std::mem::take(&mut self.deferred_mark_candidates).into_values().map(|value| value.candidate).collect();
        let chat_id = self.deferred_mark_chat_id.take();
        let Some(configuration) = self.rendered_configuration.clone() else {
            return;
        };
        if chat_id.as_deref() != Some(configuration.chat_id.as_str()) {
            return;
        }
        let Some(callback) = configuration.on_visible_mark_candidates_changed.clone() else {
            return;
        };
        self.deliver_mark_viewport(candidates, &configuration, callback, window, cx);
    }
}
