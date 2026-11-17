use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;
use gpui::{App, Context, Window, px};
use rise_core::foundation::foundation_uuid::Uuid;
use crate::core::rise_engine_bridge::rise_engine_app_runtime::RiseColdOpenTrace;
use crate::modules::chat::components::chat::async_render::chat_performance_trace::ChatPerformanceTrace;
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_types::RiseManualListTransactionAnchor;
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_view::RiseManualListPreparedViewport;
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use super::super::rise_message_list_view::*;

impl RiseMessageManualListView {
    pub(in super::super) fn initial_transaction_anchor(
        &self,
        configuration: &RiseMessageCollectionConfiguration,
        item_table: &RiseMessageManualListViewPreparedManualItemTable,
        cx: &mut App,
    ) -> RiseManualListTransactionAnchor {
        let bounds_height = f32::from(self.bounds.size.height);
        let top_inset = (configuration.top_inset + 10.0).max(10.0);
        let bottom_inset = (configuration.bottom_inset + 10.0).max(10.0);
        if let Some(request) = self.pending_scroll_request.as_ref() {
            match &request.destination {
                RiseMessageScrollDestination::Takeover => {
                    return RiseManualListTransactionAnchor::PreserveVisible;
                }
                RiseMessageScrollDestination::Latest => {
                    return if configuration.can_load_newer {
                        RiseManualListTransactionAnchor::PreserveVisible
                    } else {
                        RiseManualListTransactionAnchor::Bottom
                    };
                }
                RiseMessageScrollDestination::Message(message_id) => {
                    if let Some(entry_id) = self.message_entry_id(message_id, configuration)
                        && let Some(item) = item_table.items_by_id.get(&entry_id)
                    {
                        let visible_height = (bounds_height - top_inset - bottom_inset).max(1.0);
                        return RiseManualListTransactionAnchor::Item {
                            id: entry_id,
                            viewport_offset: top_inset + ((visible_height - item.height) * 0.5).max(0.0),
                        };
                    }
                }
                RiseMessageScrollDestination::DayStart(day_start_ms) => {
                    if let Some(entry_id) = self.date_separator_entry_id(*day_start_ms, configuration)
                        && item_table.items_by_id.contains_key(&entry_id)
                    {
                        return RiseManualListTransactionAnchor::Item {
                            id: entry_id,
                            viewport_offset: top_inset,
                        };
                    }
                }
            }
        }

        if configuration.initial_scroll_position != RiseMessageInitialScrollPosition::Bottom {
            return RiseManualListTransactionAnchor::Top;
        }
        let saved_position = configuration.saved_scroll_position(cx);
        tracing::warn!(target: "riseonly::X696", "restore key={} saved={:?} items={}", configuration.scroll_storage_key(), saved_position.as_ref().map(|position| (position.anchor_id.clone(), position.distance_from_top)), item_table.items_by_id.len());
        let Some(position) = saved_position else {
            return self
                .unread_separator_anchor(configuration, item_table)
                .unwrap_or(RiseManualListTransactionAnchor::Bottom);
        };

        if let Some(anchor_id) = &position.anchor_id {
            for candidate in RiseMessageScrollAnchorResolution::candidates(anchor_id) {
                let Some(item) = item_table.items_by_id.get(&candidate) else {
                    continue;
                };
                let minimum_offset = -(item.height - 1.0).max(0.0);
                let maximum_offset = (bounds_height - 1.0).max(0.0);
                return RiseManualListTransactionAnchor::Item {
                    id: candidate,
                    viewport_offset: position.viewport_offset.max(minimum_offset).min(maximum_offset),
                };
            }
        }

        let ids = &configuration.snapshot.ids;
        if ids.is_empty() {
            return RiseManualListTransactionAnchor::Bottom;
        }
        let spacing = MessageSizes::list_row_spacing();
        let item_height = |id: &String| item_table.items_by_id.get(id).map_or(64.0, |item| item.height);
        let content_height: f32 = ids
            .iter()
            .enumerate()
            .map(|(offset, id)| item_height(id) + if offset == 0 { 0.0 } else { spacing })
            .sum();
        let maximum_distance = (content_height - bounds_height + top_inset + bottom_inset).max(0.0);
        let distance = position.distance_from_top.max(0.0).min(maximum_distance);
        let local_viewport_top = distance - top_inset;
        let mut item_min_y = 0.0;
        for (index, id) in ids.iter().enumerate() {
            if index > 0 {
                item_min_y += spacing;
            }
            let height = item_height(id);
            if item_min_y + height > local_viewport_top || index == ids.len() - 1 {
                return RiseManualListTransactionAnchor::Item {
                    id: id.clone(),
                    viewport_offset: item_min_y - local_viewport_top,
                };
            }
            item_min_y += height;
        }
        RiseManualListTransactionAnchor::Bottom
    }

    pub(in super::super) fn schedule_initial_frame_readiness_if_needed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.schedule_initial_frame_readiness(0, window, cx);
    }

    pub(in super::super) fn schedule_initial_frame_readiness(&mut self, attempt: i64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rendered_configuration) = self.rendered_configuration.clone() else {
            return;
        };
        let source_version = rendered_configuration.snapshot.source_version;
        if self.initial_frame_ready_delivered
            || self.initial_viewport_readiness_pass_in_flight
            || self.presentation_chat_id.as_deref() != Some(rendered_configuration.chat_id.as_str())
            || self.presentation_epoch != rendered_configuration.presentation_epoch
            || self.configuration.as_ref().and_then(|configuration| configuration.presentation_epoch)
                != self.presentation_epoch
            || !rendered_configuration.initial_data_resolved
            || self.requested_source_version != Some(source_version)
            || self.applied_source_version != Some(source_version)
            || self.rendered_configuration_revision != self.configuration_revision
            || self.is_applying_snapshot
            || (self.rendered_preparation_width - f32::from(self.bounds.size.width)).abs() > 0.5
        {
            if !self.initial_frame_ready_delivered {
                let gate = format!(
                    "chat={} entries={} in_flight={} presentation_chat={:?} epoch_match={} config_epoch_match={} data_resolved={} requested={:?} applied={:?} source={source_version} revision={}/{} applying={} width={}/{}",
                    rendered_configuration.chat_id,
                    rendered_configuration.snapshot.entries.len(),
                    self.initial_viewport_readiness_pass_in_flight,
                    self.presentation_chat_id,
                    self.presentation_epoch == rendered_configuration.presentation_epoch,
                    self.configuration.as_ref().and_then(|configuration| configuration.presentation_epoch)
                        == self.presentation_epoch,
                    rendered_configuration.initial_data_resolved,
                    self.requested_source_version,
                    self.applied_source_version,
                    self.rendered_configuration_revision,
                    self.configuration_revision,
                    self.is_applying_snapshot,
                    self.rendered_preparation_width,
                    f32::from(self.bounds.size.width),
                );
                static LAST_X357_GATE: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());
                if let Ok(mut last) = LAST_X357_GATE.lock()
                    && *last != gate
                {
                    tracing::warn!(target: "riseonly::X357", "list readiness waits {gate}");
                    *last = gate;
                }
            }
            return;
        }

        self.initial_viewport_readiness_watchdog = None;
        self.initial_viewport_readiness_generation = self.initial_viewport_readiness_generation.wrapping_add(1);
        let readiness_generation = self.initial_viewport_readiness_generation;
        let expected_apply_generation = self.apply_generation;
        let expected_chat_id = rendered_configuration.chat_id.clone();
        let expected_presentation_epoch = rendered_configuration.presentation_epoch;
        let expected_width = self.rendered_preparation_width;
        let expected_configuration_revision = self.rendered_configuration_revision;
        RiseColdOpenTrace::mark(|| {
            format!("list readiness pass attempt={attempt} entries={}", rendered_configuration.snapshot.entries.len())
        });
        self.initial_viewport_readiness_pass_in_flight = true;

        let weak = cx.weak_entity();
        let completion_configuration = rendered_configuration.clone();
        let completion_chat_id = expected_chat_id.clone();
        self.list_view.update(cx, |list_view, cx| {
            list_view.prepare_visible_viewport(
                Box::new(move |viewport: RiseManualListPreparedViewport, window: &mut Window, cx: &mut App| {
                    window.defer(cx, move |window, cx| {
                        let _ = weak.update(cx, |this, cx| {
                            if this.initial_viewport_readiness_generation != readiness_generation {
                                return;
                            }
                            this.initial_viewport_readiness_pass_in_flight = false;
                            if !this.initial_readiness_state_is_current(
                                readiness_generation,
                                expected_apply_generation,
                                source_version,
                                &completion_chat_id,
                                expected_presentation_epoch,
                                expected_width,
                                expected_configuration_revision,
                            ) {
                                this.schedule_initial_frame_readiness((attempt + 1).min(2), window, cx);
                                return;
                            }
                            this.complete_initial_viewport_readiness(
                                viewport,
                                completion_configuration,
                                window,
                                cx,
                            );
                        });
                    });
                }),
                window,
                cx,
            );
        });

        self.initial_viewport_readiness_watchdog = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs_f64(0.08)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.initial_viewport_readiness_generation != readiness_generation {
                    return;
                }
                this.initial_viewport_readiness_pass_in_flight = false;
                this.initial_viewport_readiness_watchdog = None;
                if !this.initial_readiness_state_is_current(
                    readiness_generation,
                    expected_apply_generation,
                    source_version,
                    &expected_chat_id,
                    expected_presentation_epoch,
                    expected_width,
                    expected_configuration_revision,
                ) {
                    this.schedule_initial_frame_readiness((attempt + 1).min(2), window, cx);
                    return;
                }

                let viewport = this.list_view.update(cx, |list_view, cx| {
                    list_view.prepared_visible_viewport_snapshot_if_stable(window, cx)
                });
                if let Some(viewport) = viewport {
                    this.complete_initial_viewport_readiness(viewport, rendered_configuration, window, cx);
                } else if RiseMessageInitialViewportRecoveryPolicy::should_reveal_applied_geometry(attempt) {
                    ChatPerformanceTrace::shared().record(
                        "first_frame.viewport_recovery",
                        HashMap::from([
                            ("attempt".to_owned(), attempt.to_string()),
                            ("entries".to_owned(), rendered_configuration.snapshot.entries.len().to_string()),
                            ("source".to_owned(), source_version.to_string()),
                        ]),
                    );
                    let content_offset_y = this.list_view.read(cx).scroll_metrics().content_offset_y;
                    this.complete_initial_viewport_readiness(
                        RiseManualListPreparedViewport {
                            layout_generation: 0,
                            visible_ids: Vec::new(),
                            content_offset_y,
                            natural_height_mismatches: HashMap::new(),
                        },
                        rendered_configuration,
                        window,
                        cx,
                    );
                } else {
                    this.schedule_initial_frame_readiness(attempt + 1, window, cx);
                }
            });
        }));
    }

    #[allow(clippy::too_many_arguments)]
    pub(in super::super) fn initial_readiness_state_is_current(
        &self,
        readiness_generation: u64,
        expected_apply_generation: u64,
        expected_source_version: i64,
        expected_chat_id: &str,
        expected_presentation_epoch: Option<Uuid>,
        expected_width: f32,
        expected_configuration_revision: u64,
    ) -> bool {
        !self.initial_frame_ready_delivered
            && self.initial_viewport_readiness_generation == readiness_generation
            && self.apply_generation == expected_apply_generation
            && self.presentation_chat_id.as_deref() == Some(expected_chat_id)
            && self.presentation_epoch == expected_presentation_epoch
            && self.requested_source_version == Some(expected_source_version)
            && self.applied_source_version == Some(expected_source_version)
            && self.rendered_configuration.as_ref().is_some_and(|rendered| {
                rendered.chat_id == expected_chat_id && rendered.snapshot.source_version == expected_source_version
            })
            && self.rendered_configuration_revision == expected_configuration_revision
            && self.configuration_revision == expected_configuration_revision
            && self.configuration.as_ref().is_some_and(|configuration| {
                configuration.chat_id == expected_chat_id
                    && configuration.presentation_epoch == expected_presentation_epoch
                    && configuration.initial_data_resolved
            })
            && !self.is_applying_snapshot
            && (f32::from(self.bounds.size.width) - expected_width).abs() <= 0.5
    }

    pub(in super::super) fn complete_initial_viewport_readiness(
        &mut self,
        viewport: RiseManualListPreparedViewport,
        ready_configuration: Rc<RiseMessageCollectionConfiguration>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.initial_viewport_readiness_pass_in_flight = false;
        self.initial_viewport_readiness_watchdog = None;
        if !self.ensure_initial_preparation_covers_visible_viewport(&viewport, &ready_configuration, window, cx) {
            return;
        }

        if !viewport.natural_height_mismatches.is_empty() {
            let rendered_preparation_width = self.rendered_preparation_width;
            for (id, height) in &viewport.natural_height_mismatches {
                let Some(entry) = self.entries_by_id.get(id).cloned() else {
                    continue;
                };
                self.record_runtime_height_mismatch(
                    f32::from(*height),
                    &entry,
                    &ready_configuration,
                    rendered_preparation_width,
                    cx,
                );
            }
            if self.apply_runtime_height_corrections_if_possible(window, cx) {
                self.initial_viewport_height_correction_pass += 1;
                return;
            }
            ChatPerformanceTrace::shared().record(
                "first_frame.mismatch_unapplied",
                HashMap::from([
                    ("count".to_owned(), viewport.natural_height_mismatches.len().to_string()),
                    ("pass".to_owned(), self.initial_viewport_height_correction_pass.to_string()),
                    ("source".to_owned(), ready_configuration.snapshot.source_version.to_string()),
                ]),
            );
        }

        let source_version = ready_configuration.snapshot.source_version;
        if self.initial_frame_ready_delivered
            || self.presentation_chat_id.as_deref() != Some(ready_configuration.chat_id.as_str())
            || self.presentation_epoch != ready_configuration.presentation_epoch
            || self.configuration.as_ref().and_then(|configuration| configuration.presentation_epoch)
                != ready_configuration.presentation_epoch
            || self.requested_source_version != Some(source_version)
            || self.applied_source_version != Some(source_version)
        {
            return;
        }
        if self.maintains_initial_bottom_pin
            && self.initial_frame_owner.as_ref().is_some_and(|owner| {
                owner.chat_id == ready_configuration.chat_id
                    && owner.source_version == source_version
                    && owner.pins_initial_position_to_bottom
            })
        {
            let distance_bottom = self.list_view.update(cx, |list_view, cx| {
                list_view.scroll_to_bottom(false, window, cx);
                list_view.scroll_metrics().distance_from_bottom
            });
            ChatPerformanceTrace::shared().record(
                "first_frame.bottom_pinned",
                HashMap::from([
                    ("source".to_owned(), source_version.to_string()),
                    ("distance_bottom".to_owned(), f32::from(distance_bottom).to_string()),
                ]),
            );
        }
        self.initial_frame_ready_delivered = true;
        self.did_restore_initial_position = true;
        let unresolved_jump = self.pending_scroll_request.take().filter(|request| {
            matches!(
                request.destination,
                RiseMessageScrollDestination::Message(_) | RiseMessageScrollDestination::DayStart(_)
            )
        });
        self.pending_scroll_reset = unresolved_jump.is_some();
        self.pending_scroll_request = unresolved_jump;
        let queued_configuration = self.queued_pre_reveal_configuration.take();
        self.initial_frame_owner = None;
        let callback = queued_configuration
            .as_ref()
            .and_then(|queued| queued.on_initial_frame_ready.clone())
            .or_else(|| {
                match self.configuration.as_ref().filter(|current| current.chat_id == ready_configuration.chat_id) {
                    Some(current) => current.on_initial_frame_ready.clone(),
                    None => ready_configuration.on_initial_frame_ready.clone(),
                }
            });
        RiseColdOpenTrace::mark(|| {
            format!(
                "list reveal entries={} visible={}",
                ready_configuration.snapshot.entries.len(),
                viewport.visible_ids.len()
            )
        });
        let pending_reveal = RiseMessageManualListViewPendingInitialFrameReveal {
            readiness_generation: self.initial_viewport_readiness_generation,
            apply_generation: self.apply_generation,
            configuration_revision: self.rendered_configuration_revision,
            chat_id: ready_configuration.chat_id.clone(),
            source_version,
            presentation_epoch: ready_configuration.presentation_epoch,
            viewport_ids: viewport.visible_ids,
            ready_configuration,
            queued_configuration,
            callback,
        };
        self.pending_initial_frame_reveal = None;
        tracing::warn!(
            target: "riseonly::X357",
            "list reveal chat={} entries={} visible={}",
            pending_reveal.chat_id,
            pending_reveal.ready_configuration.snapshot.entries.len(),
            pending_reveal.viewport_ids.len()
        );
        self.list_alpha = 1.0;
        cx.notify();
        cx.defer_in(window, move |this, window, cx| {
            this.finish_initial_frame_reveal(pending_reveal, window, cx);
        });
        if let Some((entry_id, color)) = self.pending_reveal_highlight.take() {
            self.list_view.update(cx, |list_view, cx| {
                list_view.flash_row_highlight(&entry_id, color, 0.9, 0.6, window, cx);
            });
        }
    }

    pub(in super::super) fn ensure_initial_preparation_covers_visible_viewport(
        &mut self,
        viewport: &RiseManualListPreparedViewport,
        ready_configuration: &Rc<RiseMessageCollectionConfiguration>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let snapshot = &ready_configuration.snapshot;
        let indices: Vec<usize> =
            viewport.visible_ids.iter().filter_map(|id| snapshot.indices_by_id.get(id).copied()).collect();
        let (Some(first_visible), Some(last_visible)) = (indices.iter().min().copied(), indices.iter().max().copied())
        else {
            return true;
        };
        let required_range = RiseFallbackMeasurementPolicy::guarded_visible_range(
            first_visible as i64..last_visible as i64 + 1,
            snapshot.entries.len() as i64,
        );
        let required_range = required_range.start as usize..required_range.end as usize;
        let current_range = self.rendered_foreground_preparation_range.clone();
        if current_range.start <= required_range.start && current_range.end >= required_range.end {
            return true;
        }

        let expanded_range =
            current_range.start.min(required_range.start)..current_range.end.max(required_range.end);
        ChatPerformanceTrace::shared().record(
            "first_frame.guard_expand",
            HashMap::from([
                ("visible".to_owned(), format!("{first_visible}..<{}", last_visible + 1)),
                ("prepared".to_owned(), format!("{}..<{}", current_range.start, current_range.end)),
                ("expanded".to_owned(), format!("{}..<{}", expanded_range.start, expanded_range.end)),
            ]),
        );
        self.initial_viewport_readiness_generation = self.initial_viewport_readiness_generation.wrapping_add(1);
        self.list_view.update(cx, |list_view, _| list_view.cancel_visible_viewport_preparation());
        let prepared_configuration = ready_configuration.clone();
        self.prepare_native_layouts_in_range(
            ready_configuration.clone(),
            Some(expanded_range),
            RiseMessageManualListViewNativePreparationPurpose::Foreground,
            Box::new(move |native_batch, this, window, cx| {
                this.enqueue_prepared_snapshot(prepared_configuration, native_batch, HashMap::new(), None, None, window, cx);
            }),
            window,
            cx,
        );
        false
    }

    pub(in super::super) fn finish_initial_frame_reveal(
        &mut self,
        pending_reveal: RiseMessageManualListViewPendingInitialFrameReveal,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match RiseMessageInitialFrameRevealDeliveryPolicy::action(
            self.initial_frame_reveal_is_current(&pending_reveal),
            self.is_in_window,
        ) {
            RiseMessageInitialFrameRevealDeliveryPolicyAction::DiscardStale => {
                self.discard_stale_initial_frame_reveal(&pending_reveal, window, cx);
                return;
            }
            RiseMessageInitialFrameRevealDeliveryPolicyAction::DeferUntilWindowAttachment => {
                ChatPerformanceTrace::shared().record(
                    "first_frame.reveal_deferred",
                    HashMap::from([
                        ("source".to_owned(), pending_reveal.source_version.to_string()),
                        ("reason".to_owned(), "window_detached".to_owned()),
                    ]),
                );
                self.pending_initial_frame_reveal = Some(pending_reveal);
                return;
            }
            RiseMessageInitialFrameRevealDeliveryPolicyAction::Deliver => {}
        }
        self.pending_initial_frame_reveal = None;
        let ready_configuration = pending_reveal.ready_configuration.clone();
        let visible_ids = self.visible_ids(cx);
        let current_viewport_ids = if visible_ids.is_empty() { pending_reveal.viewport_ids.clone() } else { visible_ids };
        self.prewarm_native_text(&current_viewport_ids, window, cx);
        if let Some(callback) = pending_reveal.callback.as_ref() {
            callback(window, cx);
        }
        self.report_visible_mark_candidates(&current_viewport_ids, window, cx);
        ChatPerformanceTrace::shared().record(
            "first_frame.reveal_delivered",
            HashMap::from([("source".to_owned(), pending_reveal.source_version.to_string())]),
        );
        let metrics = self.list_view.read(cx).scroll_metrics();
        self.scroll_metrics_did_change(&metrics, window, cx);
        if self.pending_scroll_request.is_some()
            && let Some(configuration) = self
                .configuration
                .clone()
                .filter(|configuration| configuration.chat_id == ready_configuration.chat_id)
        {
            self.consume_pending_scroll_request(&configuration, window, cx);
        }
        let ready_source_version = ready_configuration.snapshot.source_version;
        match pending_reveal.queued_configuration {
            Some(queued_configuration)
                if queued_configuration.chat_id == ready_configuration.chat_id
                    && queued_configuration.snapshot.source_version != ready_source_version
                    && self.requested_source_version == Some(ready_source_version)
                    && self.configuration.as_ref().map(|configuration| configuration.snapshot.source_version)
                        == Some(ready_source_version) =>
            {
                self.apply(Rc::unwrap_or_clone(queued_configuration), window, cx);
            }
            _ => self.schedule_deferred_history_preparation_after_reveal(&ready_configuration, window, cx),
        }
    }

    pub(in super::super) fn deliver_pending_initial_frame_reveal_if_possible(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending_initial_frame_reveal) = self.pending_initial_frame_reveal.take() else {
            return;
        };
        self.finish_initial_frame_reveal(pending_initial_frame_reveal, window, cx);
    }

    pub(in super::super) fn initial_frame_reveal_is_current(
        &self,
        pending_reveal: &RiseMessageManualListViewPendingInitialFrameReveal,
    ) -> bool {
        self.initial_frame_ready_delivered
            && self.initial_viewport_readiness_generation == pending_reveal.readiness_generation
            && self.apply_generation == pending_reveal.apply_generation
            && self.configuration_revision == pending_reveal.configuration_revision
            && self.rendered_configuration_revision == pending_reveal.configuration_revision
            && self.presentation_chat_id.as_deref() == Some(pending_reveal.chat_id.as_str())
            && self.presentation_epoch == pending_reveal.presentation_epoch
            && self.requested_source_version == Some(pending_reveal.source_version)
            && self.applied_source_version == Some(pending_reveal.source_version)
            && self.rendered_configuration.as_ref().is_some_and(|rendered| {
                rendered.chat_id == pending_reveal.chat_id
                    && rendered.snapshot.source_version == pending_reveal.source_version
            })
            && self.configuration.as_ref().is_some_and(|configuration| {
                configuration.chat_id == pending_reveal.chat_id
                    && configuration.presentation_epoch == pending_reveal.presentation_epoch
            })
            && !self.is_applying_snapshot
    }

    pub(in super::super) fn discard_stale_initial_frame_reveal(
        &mut self,
        pending_reveal: &RiseMessageManualListViewPendingInitialFrameReveal,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .pending_initial_frame_reveal
            .as_ref()
            .is_some_and(|pending| pending.readiness_generation == pending_reveal.readiness_generation)
        {
            self.pending_initial_frame_reveal = None;
        }
        ChatPerformanceTrace::shared().record(
            "first_frame.reveal_stale",
            HashMap::from([("source".to_owned(), pending_reveal.source_version.to_string())]),
        );

        if self.presentation_chat_id.as_deref() != Some(pending_reveal.chat_id.as_str())
            || self.presentation_epoch != pending_reveal.presentation_epoch
            || !self.initial_frame_ready_delivered
        {
            return;
        }
        self.initial_frame_ready_delivered = false;
        self.initial_viewport_readiness_generation = self.initial_viewport_readiness_generation.wrapping_add(1);
        self.initial_viewport_readiness_pass_in_flight = false;
        self.initial_viewport_readiness_watchdog = None;
        self.initial_frame_owner = None;
        self.list_alpha = 0.0;
        cx.notify();
        self.schedule_initial_frame_readiness_if_needed(window, cx);
    }

    pub(in super::super) fn schedule_deferred_history_preparation_after_reveal(
        &mut self,
        ready_configuration: &RiseMessageCollectionConfiguration,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let expected_chat_id = ready_configuration.chat_id.clone();
        let expected_source_version = ready_configuration.snapshot.source_version;
        let expected_revision = self.rendered_configuration_revision;
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs_f64(1.0 / 60.0)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if !this.initial_frame_ready_delivered
                    || !this.rendered_configuration.as_ref().is_some_and(|rendered| {
                        rendered.chat_id == expected_chat_id
                            && rendered.snapshot.source_version == expected_source_version
                    })
                    || this.rendered_configuration_revision != expected_revision
                {
                    return;
                }
                this.begin_deferred_history_preparation_if_needed(window, cx);
            });
        })
        .detach();
    }

    pub(in super::super) fn restore_initial_position(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_user_scrolling(cx) {
            return;
        }
        let Some(configuration) = self.rendered_configuration.clone().or_else(|| self.configuration.clone()) else {
            return;
        };
        self.pending_scroll_reset = false;
        self.pending_scroll_request = None;
        self.did_restore_initial_position = true;
        let is_bottom = configuration.initial_scroll_position == RiseMessageInitialScrollPosition::Bottom;
        let saved = if is_bottom {
            configuration.saved_scroll_position(cx).map(|position| position.distance_from_top)
        } else {
            None
        };
        if is_bottom && saved.is_none() {
            self.scroll_to_bottom(window, cx);
            return;
        }
        self.list_view.update(cx, |list_view, cx| {
            let minimum_content_offset_y = list_view.scroll_metrics().minimum_content_offset_y;
            list_view.set_content_offset_y(minimum_content_offset_y + px(saved.unwrap_or(0.0)), false, true, window, cx);
        });
    }
}
