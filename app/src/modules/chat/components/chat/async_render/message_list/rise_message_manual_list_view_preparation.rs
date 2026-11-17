use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;
use gpui::{App, AppContext as _, Context, Window};
use crate::core::media::rise_media_foreground_gate::RiseMediaForegroundGate;
use crate::modules::chat::components::chat::async_render::rise_native_text_message_cell::RiseNativeTextMessageCell;
use crate::modules::chat::components::chat::async_render::async_text_bubble_predisplay::AsyncTextBubblePredisplayCache;
use crate::core::ui::clever_image_disk_cache::CleverImagePrefetchPolicy;
use super::super::rise_message_list_view::*;

impl RiseMessageManualListView {
    pub(in super::super) fn is_presentation_active_for_preparation(&self, cx: &mut App) -> bool {
        self.is_in_window
            && self.requested_animated_media_playback_active
            && RiseMediaForegroundGate::shared(cx).read(cx).is_foreground()
    }

    pub(in super::super) fn allows_speculative_preparation(&self, cx: &mut App) -> bool {
        self.is_presentation_active_for_preparation(cx) && CleverImagePrefetchPolicy::allows_prefetch_now(cx)
    }

    pub(in super::super) fn refresh_speculative_preparation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_presentation_active_for_preparation(cx) {
            self.resume_deferred_history_preparation_if_possible(window, cx);
        } else {
            self.cancel_active_deferred_history_work_for_scroll();
        }
        if !self.allows_speculative_preparation(cx) {
            self.visible_predisplay_task = None;
            self.last_visible_predisplay_center = None;
            return;
        }
        let visible_ids = self.visible_ids(cx);
        self.prewarm_native_text(&visible_ids, window, cx);
    }

    pub(in super::super) fn prewarm_native_text(&mut self, visible_ids: &[String], _window: &mut Window, cx: &mut Context<Self>) {
        if !self.allows_speculative_preparation(cx) || !self.initial_frame_ready_delivered || visible_ids.is_empty() {
            return;
        }
        let Some(configuration) = self.rendered_configuration.clone() else {
            return;
        };
        let entries = &configuration.snapshot.entries;
        let visible_id = &visible_ids[visible_ids.len() / 2];
        let Some(center) = configuration.snapshot.indices_by_id.get(visible_id).copied() else {
            return;
        };
        let previous_center = self.last_visible_predisplay_center;
        if previous_center.is_some_and(|previous_center| previous_center.abs_diff(center) < 12) {
            return;
        }
        self.last_visible_predisplay_center = Some(center);

        let moves_toward_newer = previous_center.is_none_or(|previous_center| center >= previous_center);
        let (lower_guard, upper_guard) = match (previous_center, moves_toward_newer) {
            (None, _) => (24, 24),
            (Some(_), true) => (12, 36),
            (Some(_), false) => (36, 12),
        };
        let lower = center.saturating_sub(lower_guard);
        let upper = entries.len().min(center + upper_guard + 1);
        let scale = self.display_scale;
        let cache = AsyncTextBubblePredisplayCache::shared();
        let theme = configuration.theme_store.read(cx).current_theme().clone();
        let mut inputs: Vec<RiseMessageManualListViewNativePredisplayInput> =
            Vec::with_capacity(upper.saturating_sub(lower));
        for entry in &entries[lower.min(upper)..upper] {
            if self.native_layout_versions.get(&entry.id) != Some(&entry.content_version) {
                continue;
            }
            let (Some(layout), Some(is_own), Some(group)) = (
                self.native_layouts.get(&entry.id),
                self.native_ownership_by_id.get(&entry.id).copied(),
                entry.group,
            ) else {
                continue;
            };
            if !cache.can_predisplay(layout, scale) {
                continue;
            }
            let (style, corners) = RiseNativeTextMessageCell::appearance(&theme, is_own, group.is_first, group.is_last);
            if cache.image(layout, &style, &corners, scale).is_some() {
                continue;
            }
            inputs.push(RiseMessageManualListViewNativePredisplayInput {
                prepared_layout: layout.clone(),
                style,
                corners,
                scale,
            });
        }
        if inputs.is_empty() {
            return;
        }
        let executor = cx.background_executor().clone();
        self.visible_predisplay_task = Some(cx.background_spawn(async move {
            let requests = Self::make_native_predisplay_requests(inputs);
            if requests.is_empty() {
                return;
            }
            AsyncTextBubblePredisplayCache::shared().prewarm(requests, &executor).await;
        }));
    }

    pub(in super::super) fn cancel_deferred_history_preparation(&mut self) {
        self.cancel_active_deferred_history_work_for_scroll();
        self.deferred_history_preparation_key = None;
        self.deferred_history_prepared_indices.clear();
        self.deferred_history_visible_range = None;
        self.deferred_history_last_visible_center = None;
        self.deferred_history_moves_toward_newer = false;
        self.deferred_history_needs_initial_guard = false;
    }

    pub(in super::super) fn cancel_active_deferred_history_work_for_scroll(&mut self) {
        if self.active_deferred_history_range.is_none() {
            return;
        }
        self.native_preparation_generation = self.native_preparation_generation.wrapping_add(1);
        self.native_preparation_task = None;
        self.prepared_snapshot_finalization_generation = self.prepared_snapshot_finalization_generation.wrapping_add(1);
        self.prepared_snapshot_finalization_task = None;
        self.candidate_being_predisplayed = None;
        if self.pending_prepared_candidate.as_ref().is_some_and(|candidate| {
            candidate.native_batch.purpose == RiseMessageManualListViewNativePreparationPurpose::DeferredHistory
        }) {
            self.pending_prepared_candidate = None;
        }
        self.active_deferred_history_range = None;
    }

    pub(in super::super) fn begin_deferred_history_preparation_if_needed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rendered_configuration) = self.rendered_configuration.clone() else {
            return;
        };
        if !self.initial_frame_ready_delivered
            || Some(rendered_configuration.snapshot.source_version) != self.requested_source_version
            || self.rendered_configuration_revision != self.configuration_revision
            || (self.rendered_preparation_width - f32::from(self.bounds.size.width)).abs() > 0.5
        {
            return;
        }

        let key = RiseMessageManualListViewDeferredHistoryPreparationKey {
            chat_id: rendered_configuration.chat_id.clone(),
            source_version: rendered_configuration.snapshot.source_version,
            configuration_revision: self.rendered_configuration_revision,
            width: self.rendered_preparation_width,
        };
        if self.deferred_history_preparation_key.as_ref() != Some(&key) {
            self.cancel_deferred_history_preparation();
            self.deferred_history_preparation_key = Some(key);
            let count = rendered_configuration.snapshot.entries.len();
            let lower_bound = count.min(self.rendered_foreground_preparation_range.start);
            let upper_bound = lower_bound.max(count.min(self.rendered_foreground_preparation_range.end));
            self.deferred_history_prepared_indices.extend(lower_bound..upper_bound);
            self.deferred_history_needs_initial_guard = upper_bound - lower_bound < count;
            let visible_ids = self.visible_ids(cx);
            self.update_deferred_history_viewport(&visible_ids);
        }
        self.resume_deferred_history_preparation_if_possible(window, cx);
    }

    pub(in super::super) fn update_deferred_history_viewport(&mut self, visible_ids: &[String]) {
        let Some(rendered_configuration) = self.rendered_configuration.as_ref() else {
            return;
        };
        let indices: Vec<usize> = visible_ids
            .iter()
            .filter_map(|id| rendered_configuration.snapshot.indices_by_id.get(id).copied())
            .collect();
        let (Some(lower_bound), Some(upper_index)) = (indices.iter().min().copied(), indices.iter().max().copied())
        else {
            return;
        };
        let center = lower_bound + (upper_index - lower_bound) / 2;
        if let Some(previous_center) = self.deferred_history_last_visible_center {
            self.deferred_history_moves_toward_newer = center >= previous_center;
        }
        self.deferred_history_last_visible_center = Some(center);
        self.deferred_history_visible_range = Some(lower_bound..upper_index + 1);
    }

    pub(in super::super) fn resume_deferred_history_preparation_if_possible(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_presentation_active_for_preparation(cx)
            || !self.initial_frame_ready_delivered
            || self.is_user_scrolling(cx)
            || self.is_applying_snapshot
            || self.native_preparation_task.is_some()
            || self.prepared_snapshot_finalization_task.is_some()
            || self.pending_prepared_candidate.is_some()
            || self.active_deferred_history_range.is_some()
        {
            return;
        }
        let (Some(key), Some(rendered_configuration)) =
            (self.deferred_history_preparation_key.clone(), self.rendered_configuration.clone())
        else {
            return;
        };
        if key.chat_id != rendered_configuration.chat_id
            || key.source_version != rendered_configuration.snapshot.source_version
            || key.configuration_revision != self.rendered_configuration_revision
            || (key.width - self.rendered_preparation_width).abs() > 0.5
        {
            return;
        }
        let Some(next_range) = self.next_deferred_history_range(rendered_configuration.snapshot.entries.len()) else {
            return;
        };

        self.active_deferred_history_range = Some(next_range.clone());
        let prepared_configuration = rendered_configuration.clone();
        let completion_range = next_range.clone();
        self.prepare_native_layouts_in_range(
            rendered_configuration,
            Some(next_range),
            RiseMessageManualListViewNativePreparationPurpose::DeferredHistory,
            Box::new(move |native_batch, this, window, cx| {
                if this.active_deferred_history_range.as_ref() != Some(&completion_range)
                    || this.deferred_history_preparation_key.as_ref() != Some(&key)
                {
                    return;
                }
                let fallback_heights = this.measured_heights.clone();
                this.enqueue_prepared_snapshot(
                    prepared_configuration,
                    native_batch,
                    fallback_heights,
                    Some(key.configuration_revision),
                    None,
                    window,
                    cx,
                );
            }),
            window,
            cx,
        );
    }

    pub(in super::super) fn next_deferred_history_range(&self, entry_count: usize) -> Option<Range<usize>> {
        if entry_count == 0 {
            return None;
        }
        let chunk_size = RISE_MESSAGE_MANUAL_LIST_VIEW_DEFERRED_HISTORY_CHUNK_SIZE;

        if self.deferred_history_needs_initial_guard {
            let lower_bound = entry_count.min(self.rendered_foreground_preparation_range.start);
            let upper_bound = lower_bound.max(entry_count.min(self.rendered_foreground_preparation_range.end));
            if lower_bound > 0 {
                return Some(lower_bound.saturating_sub(chunk_size)..lower_bound);
            }
            if upper_bound < entry_count {
                return Some(upper_bound..entry_count.min(upper_bound + chunk_size));
            }
            return None;
        }

        let visible_range = self.deferred_history_visible_range.clone()?;
        if let Some(unprepared) =
            visible_range.clone().find(|index| !self.deferred_history_prepared_indices.contains(index))
        {
            let limit = chunk_size.min(entry_count);
            let lower_bound = unprepared.saturating_sub(limit / 2).min(entry_count.saturating_sub(limit));
            return Some(lower_bound..lower_bound + limit);
        }

        let center = (entry_count - 1).min(visible_range.start + visible_range.len() / 2);
        if !self.deferred_history_prepared_indices.contains(&center) {
            return None;
        }
        let mut prepared_lower_bound = center;
        while prepared_lower_bound > 0 && self.deferred_history_prepared_indices.contains(&(prepared_lower_bound - 1)) {
            prepared_lower_bound -= 1;
        }
        let mut prepared_upper_bound = center + 1;
        while prepared_upper_bound < entry_count
            && self.deferred_history_prepared_indices.contains(&prepared_upper_bound)
        {
            prepared_upper_bound += 1;
        }

        let guard_count = 12_i64;
        let older_range = (visible_range.start as i64 - prepared_lower_bound as i64 <= guard_count
            && prepared_lower_bound > 0)
            .then(|| prepared_lower_bound.saturating_sub(chunk_size)..prepared_lower_bound);
        let newer_range = (prepared_upper_bound as i64 - visible_range.end as i64 <= guard_count
            && prepared_upper_bound < entry_count)
            .then(|| prepared_upper_bound..entry_count.min(prepared_upper_bound + chunk_size));
        if self.deferred_history_moves_toward_newer {
            return newer_range.or(older_range);
        }
        older_range.or(newer_range)
    }

    pub(in super::super) fn complete_deferred_history_preparation(
        &mut self,
        range: Range<usize>,
        configuration: &RiseMessageCollectionConfiguration,
        width: f32,
    ) {
        let Some(key) = self.deferred_history_preparation_key.as_ref() else {
            return;
        };
        if key.chat_id != configuration.chat_id
            || key.source_version != configuration.snapshot.source_version
            || key.configuration_revision != self.configuration_revision
            || (key.width - width).abs() > 0.5
            || self.active_deferred_history_range.as_ref() != Some(&range)
        {
            return;
        }
        self.deferred_history_prepared_indices.extend(range);
        self.deferred_history_needs_initial_guard = false;
        self.active_deferred_history_range = None;
    }

    /// retries a page parked because it wasnt scroll safe yet
    /// a candidate that failed on one frame can pass on the next so ask again under the finger
    pub(in super::super) fn retry_pending_prepared_candidate_during_scroll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_user_scrolling(cx)
            || self.is_applying_snapshot
            || self.pending_width_repreparation
            || self.prepared_snapshot_finalization_task.is_some()
            || self.holds_edge_page_for_fast_fling(cx)
        {
            return;
        }
        let Some(candidate) = self.pending_prepared_candidate.as_ref() else {
            return;
        };
        if self.scroll_safe_edge_commit(candidate, false, cx).is_none() {
            return;
        }
        let Some(candidate) = self.pending_prepared_candidate.take() else {
            return;
        };
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

    /// keeps a parked page moving while the gesture lasts
    /// at distance 0 visible rows stop changing so a clock has to drive the retry
    pub(in super::super) fn schedule_parked_candidate_retry_if_needed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.parked_candidate_retry_scheduled
            || self.pending_prepared_candidate.is_none()
            || !self.is_user_scrolling(cx)
        {
            return;
        }
        self.parked_candidate_retry_scheduled = true;
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs_f64(RiseMessageHistoryEdgePolicy::PARKED_CANDIDATE_RETRY_INTERVAL))
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.parked_candidate_retry_scheduled = false;
                this.retry_pending_prepared_candidate_during_scroll(window, cx);
                this.schedule_parked_candidate_retry_if_needed(window, cx);
            });
        })
        .detach();
    }

    /// rearming deferred work must not nest, a pass moves the scroll and that returns here
    /// a nested call is flagged and the outer call repeats the pass
    /// a pass cap stops work spawning work forever
    pub(in super::super) fn flush_deferred_work_if_possible(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.flush_deferred_mark_candidates_if_idle(window, cx);
        if self.is_flushing_deferred_work {
            self.needs_another_deferred_work_flush = true;
            return;
        }
        self.is_flushing_deferred_work = true;

        let mut passes = 0;
        loop {
            self.needs_another_deferred_work_flush = false;
            self.perform_deferred_work_pass(window, cx);
            passes += 1;
            if !self.needs_another_deferred_work_flush
                || passes >= RISE_MESSAGE_MANUAL_LIST_VIEW_DEFERRED_WORK_FLUSH_PASS_LIMIT
            {
                break;
            }
        }
        self.is_flushing_deferred_work = false;
        self.needs_another_deferred_work_flush = false;
    }

    pub(in super::super) fn perform_deferred_work_pass(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_user_scrolling(cx) || self.is_travelling(cx) || self.is_applying_snapshot {
            return;
        }
        if self.pending_width_repreparation {
            let bounds_width = f32::from(self.bounds.size.width);
            let matching_width = self
                .pending_prepared_candidate
                .as_ref()
                .map(|candidate| candidate.native_batch.width)
                .filter(|width| (width - bounds_width).abs() <= 0.5);
            if let Some(width) = matching_width {
                self.pending_width_repreparation = false;
                self.native_preparation_width = width;
            } else {
                self.pending_prepared_candidate = None;
                self.handle_width_change_if_needed(window, cx);
                return;
            }
        }
        if let Some(pending_prepared_candidate) = self.pending_prepared_candidate.take() {
            self.enqueue_prepared_snapshot(
                pending_prepared_candidate.configuration,
                pending_prepared_candidate.native_batch,
                pending_prepared_candidate.fallback_heights,
                Some(pending_prepared_candidate.configuration_revision),
                pending_prepared_candidate.history_edge_load_generation,
                window,
                cx,
            );
            return;
        }
        if self.apply_runtime_height_corrections_if_possible(window, cx) {
            return;
        }
        if self.initial_frame_ready_delivered && self.pending_scroll_reset {
            match self.configuration.clone().filter(|_| self.pending_scroll_request.is_some()) {
                Some(configuration) => self.consume_pending_scroll_request(&configuration, window, cx),
                None => self.restore_initial_position(window, cx),
            }
        }
        self.begin_deferred_history_preparation_if_needed(window, cx);
    }

    pub(in super::super) fn prepare_native_layouts(
        &mut self,
        configuration: Rc<RiseMessageCollectionConfiguration>,
        completion: RiseMessageManualListViewNativeCompletion,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prepare_native_layouts_in_range(
            configuration,
            None,
            RiseMessageManualListViewNativePreparationPurpose::Foreground,
            completion,
            window,
            cx,
        );
    }
}
