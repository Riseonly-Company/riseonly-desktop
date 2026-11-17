use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;
use gpui::{App, Context, Window};
use rise_core::foundation::foundation_date::RiseClock;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::modules::chat::components::chat::async_render::chat_performance_trace::ChatPerformanceTrace;
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_types::RiseManualListScrollMetrics;
use crate::modules::chat::engine::messages::rise_message_scope::RiseMessagePageDirection;
use crate::modules::chat::stores::message::message_actions::message_types::MessageHistoryDirection;
use crate::modules::chat::stores::message::message_services::message_pagination_trace::MessagePaginationTrace;
use super::super::rise_message_list_view::*;

impl RiseMessageManualListView {
    pub(in super::super) fn history_edge_proximity(
        edge: RiseMessageManualListViewHistoryEdge,
        metrics: &RiseManualListScrollMetrics,
    ) -> RiseMessageManualListViewHistoryEdgeProximity {
        let velocity_y = f32::from(metrics.velocity_y);
        let (distance, toward_velocity) = match edge {
            RiseMessageManualListViewHistoryEdge::Older => (f32::from(metrics.distance_from_top), (-velocity_y).max(0.0)),
            RiseMessageManualListViewHistoryEdge::Newer => (f32::from(metrics.distance_from_bottom), velocity_y.max(0.0)),
        };
        let thresholds =
            RiseMessageHistoryEdgePolicy::thresholds(f32::from(metrics.viewport_size.height), toward_velocity);
        RiseMessageManualListViewHistoryEdgeProximity {
            distance,
            load_threshold: thresholds.load,
            rearm_threshold: thresholds.rearm,
            toward_velocity,
        }
    }

    pub(in super::super) fn history_edge_entry_distance(
        &self,
        edge: RiseMessageManualListViewHistoryEdge,
        configuration: &RiseMessageCollectionConfiguration,
        cx: &App,
    ) -> Option<i64> {
        let visible_indices: Vec<usize> = self
            .list_view
            .read(cx)
            .visible_ids()
            .iter()
            .filter_map(|id| configuration.snapshot.indices_by_id.get(id).copied())
            .collect();
        let first = visible_indices.iter().min().copied()?;
        let last = visible_indices.iter().max().copied()?;
        Some(match edge {
            RiseMessageManualListViewHistoryEdge::Older => first as i64,
            RiseMessageManualListViewHistoryEdge::Newer => {
                (configuration.snapshot.entries.len() as i64 - last as i64 - 1).max(0)
            }
        })
    }

    pub(in super::super) fn evaluate_history_edge_prefetch(
        &mut self,
        metrics: &RiseManualListScrollMetrics,
        configuration: &Rc<RiseMessageCollectionConfiguration>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let velocity_y = f32::from(metrics.velocity_y);
        if self.initial_frame_ready_delivered
            && self.did_restore_initial_position
            && self.requested_animated_media_playback_active
            && self.is_in_window
            && let Some(callback) = configuration.on_prefetch_page.clone()
        {
            let older = Self::history_edge_proximity(RiseMessageManualListViewHistoryEdge::Older, metrics);
            let newer = Self::history_edge_proximity(RiseMessageManualListViewHistoryEdge::Newer, metrics);
            let direction = if velocity_y < 0.0
                && configuration.can_load_older
                && !configuration.has_cached_older
                && older.distance <= older.rearm_threshold
            {
                Some(RiseMessagePageDirection::Older)
            } else if velocity_y > 0.0
                && configuration.can_load_newer
                && !configuration.has_cached_newer
                && newer.distance <= newer.rearm_threshold
            {
                Some(RiseMessagePageDirection::Newer)
            } else {
                None
            };
            if self.remote_prefetch_direction != direction
                || self.remote_prefetch_source_version != Some(configuration.snapshot.source_version)
            {
                self.remote_prefetch_direction = direction;
                self.remote_prefetch_source_version = Some(configuration.snapshot.source_version);
                callback(direction, window, cx);
            }
        }
        let viewport_is_underfilled = f32::from(metrics.viewport_size.height) > 1.0
            && f32::from(metrics.maximum_content_offset_y) <= f32::from(metrics.minimum_content_offset_y) + 1.0;
        let x2021_near_top = velocity_y < 0.0
            && f32::from(metrics.distance_from_top) <= f32::from(metrics.viewport_size.height);
        if x2021_near_top {
            tracing::warn!(
                target: "riseonly::X2021",
                chat = %configuration.chat_id,
                distance = f32::from(metrics.distance_from_top),
                can_load_older = configuration.can_load_older,
                has_cached_older = configuration.has_cached_older,
                has_older_callback = configuration.on_load_older.is_some(),
                restored = self.did_restore_initial_position,
                frame_ready = self.initial_frame_ready_delivered,
                observed_scroll = self.has_observed_history_scroll_interaction,
                pending_commit = self.pending_presentation_commit.is_some(),
                applying = self.is_applying_snapshot,
                tracking = metrics.is_tracking,
                dragging = metrics.is_dragging,
                decelerating = metrics.is_decelerating,
                requested_version = ?self.requested_source_version,
                applied_version = ?self.applied_source_version,
                preparing = self.pending_prepared_candidate.is_some()
                    || self.prepared_snapshot_finalization_task.is_some()
                    || self.native_preparation_task.is_some(),
                armed = self.load_older_armed,
                pending_edge = self.pending_history_edge_load.is_some(),
                "history edge near top"
            );
        }
        if !RiseMessageHistoryEdgePolicy::can_evaluate_prefetch(
            self.did_restore_initial_position,
            !configuration.snapshot.entries.is_empty(),
            self.initial_frame_ready_delivered,
            self.has_observed_history_scroll_interaction,
            viewport_is_underfilled,
            self.pending_presentation_commit.is_some(),
            self.is_applying_snapshot,
            metrics.is_tracking,
            metrics.is_dragging,
            metrics.is_decelerating,
        ) {
            return;
        }
        if !RiseMessageHistoryEdgePolicy::rendered_window_is_current(
            self.requested_source_version,
            self.applied_source_version,
            self.pending_prepared_candidate.is_some()
                || self.prepared_snapshot_finalization_task.is_some()
                || self.native_preparation_task.is_some()
                || self.is_applying_snapshot,
        ) {
            return;
        }

        let now = RiseClock::now().time_interval_since_1970();
        let is_moving_toward_older = velocity_y < 0.0;
        let is_moving_toward_newer = velocity_y > 0.0;
        let ignores_direction = viewport_is_underfilled || (!is_moving_toward_older && !is_moving_toward_newer);
        let may_request_older_edge = ignores_direction || is_moving_toward_older;
        let may_request_newer_edge = ignores_direction || is_moving_toward_newer;
        if is_moving_toward_older || is_moving_toward_newer {
            self.last_history_motion_toward_older = Some(is_moving_toward_older);
        }

        let older_proximity = Self::history_edge_proximity(RiseMessageManualListViewHistoryEdge::Older, metrics);
        let older_entry_distance =
            self.history_edge_entry_distance(RiseMessageManualListViewHistoryEdge::Older, configuration, cx);
        let older_inside_load_zone = RiseMessageHistoryEdgePolicy::is_inside_load_zone(
            older_proximity.distance,
            older_proximity.load_threshold,
            configuration.has_cached_older,
            older_entry_distance,
            RiseMessageHistoryEdgePolicy::heads_toward_edge(true, velocity_y, self.last_history_motion_toward_older),
        );
        if RiseMessageHistoryEdgePolicy::is_outside_rearm_zone(
            older_proximity.distance,
            older_proximity.rearm_threshold,
            configuration.has_cached_older,
            older_entry_distance,
        ) {
            self.load_older_armed = true;
        }
        let can_request_older = RiseMessageHistoryEdgePolicy::can_request(
            self.load_older_armed,
            self.pending_history_edge_load.is_some(),
            configuration.can_load_older,
            configuration.on_load_older.is_some(),
            now - self.last_load_older_at,
        );
        if x2021_near_top {
            tracing::warn!(
                target: "riseonly::X2021",
                can_request_older,
                older_inside_load_zone,
                may_request_older_edge,
                load_threshold = older_proximity.load_threshold,
                entry_distance = ?older_entry_distance,
                since_last = now - self.last_load_older_at,
                "history edge older decision"
            );
        }
        if can_request_older && older_inside_load_zone && may_request_older_edge {
            self.last_load_older_at = now;
            self.load_older_armed = false;
            let anchor_id = self.persisted_visible_anchor(cx).map(|anchor| anchor.id);
            self.request_history_edge(
                RiseMessageManualListViewHistoryEdge::Older,
                configuration,
                anchor_id,
                &older_proximity,
                metrics,
                window,
                cx,
            );
        }

        let newer_proximity = Self::history_edge_proximity(RiseMessageManualListViewHistoryEdge::Newer, metrics);
        let newer_entry_distance =
            self.history_edge_entry_distance(RiseMessageManualListViewHistoryEdge::Newer, configuration, cx);
        let newer_inside_load_zone = RiseMessageHistoryEdgePolicy::is_inside_load_zone(
            newer_proximity.distance,
            newer_proximity.load_threshold,
            configuration.has_cached_newer,
            newer_entry_distance,
            RiseMessageHistoryEdgePolicy::heads_toward_edge(false, velocity_y, self.last_history_motion_toward_older),
        );
        if RiseMessageHistoryEdgePolicy::is_outside_rearm_zone(
            newer_proximity.distance,
            newer_proximity.rearm_threshold,
            configuration.has_cached_newer,
            newer_entry_distance,
        ) {
            self.load_newer_armed = true;
        }
        let can_request_newer = RiseMessageHistoryEdgePolicy::can_request(
            self.load_newer_armed,
            self.pending_history_edge_load.is_some(),
            configuration.can_load_newer,
            configuration.on_load_newer.is_some(),
            now - self.last_load_newer_at,
        );
        if can_request_newer && newer_inside_load_zone && may_request_newer_edge {
            self.last_load_newer_at = now;
            self.load_newer_armed = false;
            let anchor_id = self.persisted_visible_anchor(cx).map(|anchor| anchor.id);
            self.request_history_edge(
                RiseMessageManualListViewHistoryEdge::Newer,
                configuration,
                anchor_id,
                &newer_proximity,
                metrics,
                window,
                cx,
            );
        }

        if !configuration.can_load_older {
            self.load_older_armed = true;
        }
        if !configuration.can_load_newer {
            self.load_newer_armed = true;
        }
    }

    pub(in super::super) fn invalidate_history_edge_for_pending_presentation(&mut self) {
        self.has_observed_history_scroll_interaction = false;
        self.last_history_motion_toward_older = None;
        self.history_edge_load_generation = self.history_edge_load_generation.wrapping_add(1);
        self.clear_pending_history_edge_load();
        self.load_older_armed = true;
        self.last_load_older_at = 0.0;
        self.load_newer_armed = true;
        self.last_load_newer_at = 0.0;
    }

    pub(in super::super) fn clear_pending_presentation_commit(
        &mut self,
        configuration: &RiseMessageCollectionConfiguration,
        did_commit: bool,
    ) {
        let committed = RiseMessagePendingPresentationCommit {
            account_user_id: SaiAccountCacheScope::active_user_id(),
            account_epoch: SaiAccountCacheScope::active_epoch(),
            chat_id: configuration.chat_id.clone(),
            presentation_epoch: configuration.presentation_epoch,
            source_version: configuration.snapshot.source_version,
        };
        if !RiseMessagePendingPresentationCommitPolicy::should_clear_after_commit(
            self.pending_presentation_commit.as_ref(),
            &committed,
            did_commit,
        ) {
            return;
        }
        self.pending_presentation_commit = None;
    }

    pub(in super::super) fn edge_trace_detail(
        proximity: &RiseMessageManualListViewHistoryEdgeProximity,
        metrics: &RiseManualListScrollMetrics,
    ) -> String {
        format!(
            "distance={} threshold={} velocity={} tracking={} dragging={} decelerating={}",
            proximity.distance.round() as i64,
            proximity.load_threshold.round() as i64,
            proximity.toward_velocity.round() as i64,
            metrics.is_tracking,
            metrics.is_dragging,
            metrics.is_decelerating,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(in super::super) fn request_history_edge(
        &mut self,
        edge: RiseMessageManualListViewHistoryEdge,
        configuration: &Rc<RiseMessageCollectionConfiguration>,
        anchor_id: Option<String>,
        proximity: &RiseMessageManualListViewHistoryEdgeProximity,
        metrics: &RiseManualListScrollMetrics,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (callback, is_cached_shift, direction, direction_name) = match edge {
            RiseMessageManualListViewHistoryEdge::Older => (
                configuration.on_load_older.clone(),
                configuration.has_cached_older,
                MessageHistoryDirection::Older,
                "older",
            ),
            RiseMessageManualListViewHistoryEdge::Newer => (
                configuration.on_load_newer.clone(),
                configuration.has_cached_newer,
                MessageHistoryDirection::Newer,
                "newer",
            ),
        };
        let Some(callback) = callback else {
            self.arm_history_edge(edge);
            return;
        };

        self.history_edge_load_generation = self.history_edge_load_generation.wrapping_add(1);
        let pending = RiseMessageManualListViewPendingHistoryEdgeLoad {
            edge,
            chat_id: configuration.chat_id.clone(),
            source_version: configuration.snapshot.source_version,
            generation: self.history_edge_load_generation,
            is_cached_shift,
        };
        self.pending_history_edge_load = Some(pending.clone());
        self.pending_history_edge_progress_generation = None;
        let trace_detail = Self::edge_trace_detail(proximity, metrics);
        MessagePaginationTrace::record(
            "edge.trigger",
            &configuration.chat_id,
            Some(direction),
            None,
            None,
            anchor_id.as_deref(),
            None,
            None,
            Some(configuration.snapshot.entries.len() as i64),
            None,
            Some(&trace_detail),
            cx,
        );
        MessagePaginationTrace::diagnostic(
            "edge.trigger",
            &configuration.chat_id,
            Some(direction),
            None,
            &format!("{trace_detail} anchor={} cached={is_cached_shift}", anchor_id.as_deref().unwrap_or("none")),
            0.0,
            cx,
        );
        let entry_distance = self
            .history_edge_entry_distance(edge, configuration, cx)
            .map_or_else(|| "unknown".to_owned(), |distance| distance.to_string());
        ChatPerformanceTrace::shared().record(
            "pagination.edge_trigger",
            HashMap::from([
                ("direction".to_owned(), direction_name.to_owned()),
                ("distance".to_owned(), (proximity.distance.round() as i64).to_string()),
                ("threshold".to_owned(), (proximity.load_threshold.round() as i64).to_string()),
                ("entry_distance".to_owned(), entry_distance),
                ("cached".to_owned(), if is_cached_shift { "1" } else { "0" }.to_owned()),
                ("snapshot_applying".to_owned(), if self.is_applying_snapshot { "1" } else { "0" }.to_owned()),
                ("tracking".to_owned(), if metrics.is_tracking { "1" } else { "0" }.to_owned()),
                ("dragging".to_owned(), if metrics.is_dragging { "1" } else { "0" }.to_owned()),
                ("decelerating".to_owned(), if metrics.is_decelerating { "1" } else { "0" }.to_owned()),
            ]),
        );

        let is_current = self.pending_history_edge_load.as_ref() == Some(&pending)
            && self.presentation_chat_id.as_deref() == Some(pending.chat_id.as_str())
            && self.rendered_configuration.as_ref().map(|rendered| rendered.snapshot.source_version)
                == Some(pending.source_version);
        if !RiseMessageHistoryEdgePolicy::deliver_request_if_current(is_current, || {
            callback(anchor_id.as_deref(), window, cx);
        }) {
            let rendered_source_version =
                self.rendered_configuration.as_ref().map_or(0, |rendered| rendered.snapshot.source_version);
            MessagePaginationTrace::diagnostic(
                "edge.reject",
                &configuration.chat_id,
                Some(direction),
                None,
                &format!("reason=stale source={} rendered={rendered_source_version}", pending.source_version),
                0.0,
                cx,
            );
            self.cancel_pending_history_edge(&pending, false, window, cx);
            return;
        }

        let watchdog_delay = RiseMessageHistoryEdgePolicy::pending_watchdog_delay(pending.is_cached_shift);
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs_f64(watchdog_delay)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.cancel_pending_history_edge(&pending, true, window, cx);
            });
        })
        .detach();
    }

    pub(in super::super) fn cancel_pending_history_edge(
        &mut self,
        pending: &RiseMessageManualListViewPendingHistoryEdgeLoad,
        callback_was_invoked: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending_history_edge_load.as_ref() != Some(pending) {
            return;
        }
        let observed_progress = self.pending_history_edge_progress_generation == Some(pending.generation);
        let direction = match pending.edge {
            RiseMessageManualListViewHistoryEdge::Older => MessageHistoryDirection::Older,
            RiseMessageManualListViewHistoryEdge::Newer => MessageHistoryDirection::Newer,
        };
        MessagePaginationTrace::diagnostic(
            "edge.watchdog",
            &pending.chat_id,
            Some(direction),
            None,
            &format!(
                "callback={callback_was_invoked} progress={observed_progress} cached={}",
                pending.is_cached_shift
            ),
            0.0,
            cx,
        );
        self.clear_pending_history_edge_load();
        match pending.edge {
            RiseMessageManualListViewHistoryEdge::Older => {
                self.last_load_older_at = RiseMessageHistoryEdgePolicy::request_timestamp_after_cancellation(
                    self.last_load_older_at,
                    callback_was_invoked,
                );
            }
            RiseMessageManualListViewHistoryEdge::Newer => {
                self.last_load_newer_at = RiseMessageHistoryEdgePolicy::request_timestamp_after_cancellation(
                    self.last_load_newer_at,
                    callback_was_invoked,
                );
            }
        }
        self.arm_history_edge(pending.edge);
        if !RiseMessageHistoryEdgePolicy::should_reevaluate_after_cancellation(callback_was_invoked, observed_progress) {
            return;
        }
        cx.defer_in(window, |this, window, cx| {
            let metrics = this.list_view.read(cx).scroll_metrics();
            this.scroll_metrics_did_change(&metrics, window, cx);
        });
    }

    pub(in super::super) fn clear_pending_history_edge_load(&mut self) {
        self.pending_history_edge_load = None;
        self.pending_history_edge_progress_generation = None;
    }

    pub(in super::super) fn arm_history_edge(&mut self, edge: RiseMessageManualListViewHistoryEdge) {
        match edge {
            RiseMessageManualListViewHistoryEdge::Older => self.load_older_armed = true,
            RiseMessageManualListViewHistoryEdge::Newer => self.load_newer_armed = true,
        }
    }

    /// a page under a fast fling that cant reach the edge it heads for waits
    pub(in super::super) fn holds_edge_page_for_fast_fling(&self, cx: &App) -> bool {
        if !self.is_user_scrolling(cx) {
            return false;
        }
        let metrics = self.list_view.read(cx).scroll_metrics();
        let velocity_y = f32::from(metrics.velocity_y);
        let distance = if velocity_y < 0.0 {
            f32::from(metrics.distance_from_top)
        } else {
            f32::from(metrics.distance_from_bottom)
        };
        RiseMessageEdgePagePacingPolicy::holds(velocity_y.abs(), distance, f32::from(metrics.viewport_size.height))
    }
}
