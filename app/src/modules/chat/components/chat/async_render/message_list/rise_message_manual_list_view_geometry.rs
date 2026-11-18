use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash as _, Hasher as _};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use futures::channel::oneshot;
use gpui::{App, AppContext as _, AsyncWindowContext, Context, WeakEntity, Window};
use rise_core::foundation::foundation_date::RiseClock;
use crate::core::rise_engine_bridge::rise_engine_app_runtime::RiseColdOpenTrace;
use crate::core::ui::formatted_text_ui::{FormattedText, FormattedTextInlinePreparation, FormattedTextOptions, FormattedTextRenderSeed};
use crate::modules::chat::components::chat::async_render::rise_async_text_message_row::RiseAsyncTextMessageEligibility;
use crate::modules::chat::components::chat::async_render::async_text_bubble_preparer::AsyncTextBubblePreparer;
use crate::modules::chat::components::chat::async_render::async_text_bubble_prepared_layout_cache::AsyncTextBubblePreparedLayoutCache;
use crate::modules::chat::components::chat::async_render::rise_message_fallback_height_cache::RiseMessageFallbackGeometryContext;
use crate::modules::chat::pages::chats::chat::chat_roles::role_mention_composer::RoleMentionComposer;
use crate::modules::chat::pages::chats::chat::chat_roles::topic_mention_composer::TopicMentionComposer;
use rise_ui::rich_text::rich_text_markup::RichTextMarkup;
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_types::RiseManualListReusableView as _;
use rise_canon::rise_key::otp as rise_otp_key;
use crate::modules::chat::components::chat::async_render::async_text_bubble_types::AsyncTextBubblePreparedLayout;
use crate::modules::chat::components::chat::async_render::chat_message_render_snapshot::{ChatMessageRenderEntry, ChatMessageRenderEntryKind};
use crate::modules::chat::components::chat::async_render::chat_performance_trace::ChatPerformanceTrace;
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_types::{RiseManualListTransactionAnchor, RiseManualListTransactionEdgeMutation};
use crate::modules::chat::components::chat::async_render::rise_manual_message_hosting_view::RiseManualMessageHostingView;
use crate::modules::chat::components::chat::async_render::rise_message_fallback_height_cache::RiseMessageFallbackHeightCache;
use crate::modules::chat::components::chat::async_render::rise_native_system_row_view::RiseNativeSystemRowView;
use crate::modules::chat::components::chat::bubble::message_sticker_view::MessageStickerLayout;
use crate::modules::chat::components::chat::bubble::message_call_view::MessageCallBubbleGeometry;
use crate::modules::chat::components::chat::bubble::message_content::{MessageContent, MessageContentKind};
use crate::modules::chat::components::chat::message_list::MessageList;
use crate::modules::chat::components::chat::async_render::list_engine::rise_list_jump_debug::RiseListJumpDebug;
use crate::modules::chat::components::chat::async_render::rise_exact_fallback_geometry::RiseExactFallbackGeometry;
use crate::modules::chat::components::chat::video_note::video_note_message_view::VideoNoteMessageView;
use crate::modules::chat::components::chat::bubble::bot_message_inline_keyboard::BotInlineKeyboardLayout;
use crate::modules::chat::components::chat::bubble::message_content::TelegramReactionLayout;
use crate::modules::chat::shared::chat_saved_message_presentation::ChatSavedMessagePresentation;
use crate::modules::chat::shared::chat_search_highlight::ChatMessageSearchHighlight;
use crate::modules::chat::shared::config::message_sizes::{MessageAccessoryStackGeometry, MessageSizes};
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType};
use crate::modules::chat::stores::message::message_actions::message_types::MessageContentType;
use crate::modules::chat::stores::message::message_services::message_pagination_trace::MessagePaginationTrace;
use crate::modules::chat::components::chat::native_bubbles::native_bubble_registry::NativeBubbleRegistry;
use super::super::rise_message_list_view::*;

impl RiseMessageManualListView {
    /// a programmatic travel like the down button or a jump owns the offset
    /// a commit prepared under it resolves against a viewport still in motion and would snap rows back
    /// so such work waits for the landing
    pub(in super::super) fn is_travelling(&self, cx: &App) -> bool {
        self.list_view.read(cx).is_programmatic_scroll_in_flight()
    }

    pub(in super::super) fn scroll_safe_environment_is_unchanged(
        previous: &RiseMessageCollectionConfiguration,
        next: &RiseMessageCollectionConfiguration,
    ) -> bool {
        previous.chat_id == next.chat_id
            && Self::cell_configuration_version(&previous.chat) == Self::cell_configuration_version(&next.chat)
            && previous.theme_palette == next.theme_palette
            && !rise_message_rendered_environment_differs(previous, next)
            && previous.interactions_enabled == next.interactions_enabled
            && previous.allows_message_selection == next.allows_message_selection
    }

    pub(in super::super) fn preparation_purpose_trace_name(purpose: RiseMessageManualListViewNativePreparationPurpose) -> &'static str {
        match purpose {
            RiseMessageManualListViewNativePreparationPurpose::Foreground => "foreground",
            RiseMessageManualListViewNativePreparationPurpose::DeferredHistory => "deferred",
        }
    }

    pub(in super::super) fn prepared_row_height(
        &self,
        entry: &ChatMessageRenderEntry,
        candidate: &RiseMessageManualListViewPreparedSnapshotCandidate,
        window: &mut Window,
        cx: &mut App,
    ) -> f32 {
        let width = candidate.native_batch.width;
        if entry.kind != ChatMessageRenderEntryKind::Message {
            return RiseNativeSystemRowView::measure(entry, width, window);
        }
        if candidate.native_batch.versions.get(&entry.id) == Some(&entry.content_version)
            && let (Some(layout), Some(group)) = (candidate.native_batch.layouts.get(&entry.id), entry.group)
        {
            let (top, bottom) = rise_message_row_vertical_paddings(group);
            return (top + f32::from(layout.size.height) + bottom).ceil();
        }
        let fallback_height = match candidate.fallback_heights.get(&entry.id).copied() {
            Some(height) => height,
            None => self.estimated_row_height(entry, &candidate.configuration, width, window, cx),
        };
        fallback_height + self.video_note_growth(entry, &candidate.configuration, width, cx)
    }

    pub(in super::super) fn priority_preparation_range(&self, configuration: &RiseMessageCollectionConfiguration) -> Range<usize> {
        let range = RiseFallbackMeasurementPolicy::initial_preparation_range(configuration.snapshot.entries.len() as i64);
        range.start as usize..range.end as usize
    }

    pub(in super::super) fn estimated_row_height(
        &self,
        entry: &ChatMessageRenderEntry,
        configuration: &RiseMessageCollectionConfiguration,
        width: f32,
        window: &mut Window,
        cx: &mut App,
    ) -> f32 {
        let (Some(message), Some(group)) = (entry.message.as_ref(), entry.group) else {
            return RiseNativeSystemRowView::measure(entry, width, window);
        };
        if entry.kind != ChatMessageRenderEntryKind::Message {
            return RiseNativeSystemRowView::measure(entry, width, window);
        }
        let chat = &configuration.chat;
        let is_own = MessageList::is_own_bubble(message, chat, cx);
        let bubble_width = MessageSizes::available_bubble_width(width, chat, is_own);
        let text_width = (bubble_width - MessageSizes::horizontal_bubble_padding(width) * 2.0).max(56.0);
        let characters_per_line = ((text_width / 8.2) as i64).max(8);
        let text_units = (message.text.chars().count() as i64).max(1);
        let estimated_lines = ((text_units as f64 / characters_per_line as f64).ceil() as i64).max(1);
        let text_height = estimated_lines as f32 * 20.0;
        let text_addition = |extra: f32| if message.has_text() { text_height + extra } else { 0.0 };

        let kind = MessageContent::bubble_kind(message, Some(chat.as_ref()));
        let body_height = match kind {
            MessageContentKind::Text => text_height + 34.0,
            MessageContentKind::MediaGrid => 300.0 + text_addition(18.0),
            MessageContentKind::Voice => 95.0,
            MessageContentKind::Audio => 95.0 + text_addition(8.0),
            MessageContentKind::Document => {
                let extra_rows = message.document_items().len().saturating_sub(1) as f32;
                110.0 + extra_rows * (54.0 + MessageContent::DOCUMENT_ROW_GAP) + text_addition(8.0)
            }
            MessageContentKind::Contact => 112.0,
            MessageContentKind::Location => 250.0,
            MessageContentKind::Poll => 220.0,
            MessageContentKind::Call => MessageCallBubbleGeometry::BUBBLE_HEIGHT,
            MessageContentKind::Sticker => {
                message
                    .first_sticker_item()
                    .map_or(184.0, |item| f32::from(MessageStickerLayout::display_size(item).height))
                    + MessageStickerLayout::META_OVERHANG
            }
            MessageContentKind::VideoNote => {
                VideoNoteMessageView::expanded_size(width, MessageSizes::reserves_avatar_space(chat, is_own)) + 21.0
            }
            MessageContentKind::PostShort => 190.0,
        };

        let mut chrome_height = 0.0;
        if !is_own && group.is_first && (chat.r#type == ChatType::Group || chat.r#type == ChatType::Favourites) {
            chrome_height += 22.0;
        } else if message.via_bot_label_tag().is_some() {
            chrome_height += 22.0;
        }
        if message.forward_info.is_some() && !ChatSavedMessagePresentation::hides_forward_header(message, chat, cx) {
            chrome_height += 30.0;
        }
        if message.reply_to.is_some() {
            chrome_height += 38.0;
        }
        if message.story_reply.is_some() {
            chrome_height += 38.0;
        }
        if message.link_preview.is_some() {
            chrome_height += 150.0;
        }
        let embedded_reaction_height = if message.has_reaction_row() && message.content_type != MessageContentType::Call {
            let has_bubble_tail =
                kind != MessageContentKind::Sticker && kind != MessageContentKind::VideoNote;
            let reaction_insets = if has_bubble_tail {
                MessageSizes::bubble_tail_inset() + if is_own { 0.0 } else { 2.0 }
            } else {
                0.0
            };
            Some(TelegramReactionLayout::estimated_height(
                &message.reactions,
                bubble_width - reaction_insets,
                is_own,
                current_user_id(cx).as_deref(),
                message.sparks_total,
                chat.r#type == ChatType::Channel,
            ))
        } else {
            None
        };

        let inline_keyboard_row_count = message.reply_markup.as_ref().map_or(0, |markup| {
            markup.inline_keyboard.iter().filter(|row| !row.is_empty()).count()
        });
        let inline_keyboard_height =
            (inline_keyboard_row_count > 0).then(|| BotInlineKeyboardLayout::height(inline_keyboard_row_count));
        chrome_height += MessageAccessoryStackGeometry::additional_height(inline_keyboard_height, embedded_reaction_height);

        if chat.r#type == ChatType::Channel
            && !message.is_system()
            && message.discussion_chat_id.as_deref().is_some_and(|id| !id.is_empty())
        {
            chrome_height += 43.0;
        }

        let (top, bottom) = rise_message_row_vertical_paddings(group);
        let row_body = if MessageSizes::reserves_avatar_space(chat, is_own) {
            MessageSizes::avatar_size().max(body_height + chrome_height)
        } else {
            body_height + chrome_height
        };
        (top + row_body + bottom).max(44.0).ceil()
    }

    pub(in super::super) fn has_prepared_native_geometry(
        index: usize,
        candidate: &RiseMessageManualListViewPreparedSnapshotCandidate,
    ) -> bool {
        let entry = &candidate.configuration.snapshot.entries[index];
        candidate.native_batch.versions.get(&entry.id) == Some(&entry.content_version)
            && candidate.native_batch.layouts.contains_key(&entry.id)
            && candidate.native_batch.ownership_by_id.contains_key(&entry.id)
    }

    pub(in super::super) fn scroll_safe_fallback_geometry_indices(
        &self,
        candidate: &RiseMessageManualListViewPreparedSnapshotCandidate,
        mutation: &RiseManualListTransactionEdgeMutation,
    ) -> Vec<usize> {
        self.scroll_safe_fallback_geometry_indices_for_native_ids(candidate, mutation, None)
    }

    pub(in super::super) fn scroll_safe_fallback_geometry_indices_for_native_ids(
        &self,
        candidate: &RiseMessageManualListViewPreparedSnapshotCandidate,
        mutation: &RiseManualListTransactionEdgeMutation,
        native_ids: Option<&HashSet<String>>,
    ) -> Vec<usize> {
        let entries = &candidate.configuration.snapshot.entries;
        let lower_retained_bound = mutation.inserted_prefix_count;
        let Some(upper_retained_bound) = entries.len().checked_sub(mutation.inserted_suffix_count) else {
            return Vec::new();
        };
        if lower_retained_bound > upper_retained_bound {
            return Vec::new();
        }

        let mut result: Vec<usize> =
            Vec::with_capacity(mutation.inserted_prefix_count + mutation.inserted_suffix_count + 2);
        let mut included: HashSet<usize> = HashSet::new();
        let uses_native_geometry = |index: usize| -> bool {
            if !Self::has_prepared_native_geometry(index, candidate) {
                return false;
            }
            native_ids.is_none_or(|native_ids| native_ids.contains(&entries[index].id))
        };
        let mut include_fallback_if_needed = |index: usize| {
            if entries[index].kind != ChatMessageRenderEntryKind::Message
                || uses_native_geometry(index)
                || included.contains(&index)
            {
                return;
            }
            result.push(index);
            included.insert(index);
        };

        for index in 0..lower_retained_bound {
            include_fallback_if_needed(index);
        }
        for index in upper_retained_bound..entries.len() {
            include_fallback_if_needed(index);
        }
        for index in lower_retained_bound..upper_retained_bound {
            let entry = &entries[index];
            if self.versions_by_id.get(&entry.id) == Some(&entry.content_version) {
                continue;
            }
            include_fallback_if_needed(index);
        }
        result
    }

    pub(in super::super) fn scroll_safe_geometry_is_complete(
        &self,
        candidate: &RiseMessageManualListViewPreparedSnapshotCandidate,
        mutation: &RiseManualListTransactionEdgeMutation,
        native_ids: Option<&HashSet<String>>,
    ) -> bool {
        self.scroll_safe_fallback_geometry_indices_for_native_ids(candidate, mutation, native_ids)
            .into_iter()
            .all(|index| {
                let id = &candidate.configuration.snapshot.entries[index].id;
                RiseMessagePreparedGeometryPolicy::has_exact_fallback_height(
                    candidate.fallback_heights.get(id).copied(),
                )
            })
    }

    pub(in super::super) fn scroll_safe_critical_geometry_indices(
        &self,
        candidate: &RiseMessageManualListViewPreparedSnapshotCandidate,
    ) -> Vec<usize> {
        let Some(previous_configuration) = self.rendered_configuration.as_ref() else {
            return Vec::new();
        };
        if previous_configuration.chat_id != candidate.configuration.chat_id {
            return Vec::new();
        }
        let old_ids = &previous_configuration.snapshot.ids;
        let new_ids = &candidate.configuration.snapshot.ids;
        if old_ids == new_ids {
            return Vec::new();
        }
        let Some(mutation) = Self::edge_mutation(old_ids, new_ids) else {
            return Vec::new();
        };
        self.scroll_safe_fallback_geometry_indices(candidate, &mutation)
    }

    pub(in super::super) fn record_runtime_height_mismatch(
        &mut self,
        raw_height: f32,
        entry: &ChatMessageRenderEntry,
        measured_configuration: &Rc<RiseMessageCollectionConfiguration>,
        width: f32,
        cx: &mut Context<Self>,
    ) {
        if let Some(message) = entry.message.as_ref()
            && (self.grown_video_note_message_id.as_deref() == Some(message.id.as_str())
                || Self::playing_video_note_message_id(cx).as_deref() == Some(message.id.as_str()))
        {
            return;
        }
        let Some(current_configuration) = self.rendered_configuration.clone() else {
            return;
        };
        if !raw_height.is_finite()
            || raw_height <= 0.0
            || current_configuration.chat_id != measured_configuration.chat_id
            || self.requested_source_version != Some(current_configuration.snapshot.source_version)
            || current_configuration.snapshot.versions_by_id.get(&entry.id) != Some(&entry.content_version)
            || self.versions_by_id.get(&entry.id) != Some(&entry.content_version)
            || !Self::scroll_safe_environment_is_unchanged(measured_configuration, &current_configuration)
            || (self.rendered_preparation_width - width).abs() > 0.5
            || (f32::from(self.bounds.size.width) - width).abs() > 0.5
        {
            return;
        }

        let display_scale = self.display_scale;
        let pixel_scale = if display_scale.is_finite() { display_scale.max(1.0) } else { 1.0 };
        let height = (raw_height * pixel_scale).ceil() / pixel_scale;
        let planned_height = self
            .measured_heights
            .get(&entry.id)
            .copied()
            .or_else(|| self.manual_items_by_id.get(&entry.id).map(|item| item.height))
            .unwrap_or(0.0);
        let tolerance = (1.0 / pixel_scale).max(0.5);
        if (height - planned_height).abs() <= tolerance {
            return;
        }

        let is_visible = self.list_view.read(cx).visible_ids().contains(&entry.id);
        let content_kind =
            entry.message.as_ref().map_or_else(|| "system".to_owned(), |message| message.content_type.raw_value().to_owned());
        if RiseListJumpDebug::is_enabled() {
            let is_own = entry
                .message
                .as_ref()
                .map(|message| MessageList::is_own_bubble(message, &current_configuration.chat, cx));
            let message = format!(
                "HEIGHT-MISMATCH id={} kind={:?} content={} own={:?} first={:?} last={:?} planned={} actual={} delta={} visible={} scrolling={}",
                entry.id,
                entry.kind,
                content_kind,
                is_own,
                entry.group.map(|group| group.is_first),
                entry.group.map(|group| group.is_last),
                RiseListJumpDebug::f(planned_height),
                RiseListJumpDebug::f(height),
                RiseListJumpDebug::f(height - planned_height),
                is_visible,
                self.is_user_scrolling(cx),
            );
            RiseListJumpDebug::log(&message, cx);
        }

        ChatPerformanceTrace::shared().record_height_mismatch(
            &entry.id,
            &content_kind,
            planned_height,
            height,
            is_visible,
            current_configuration.snapshot.source_version,
        );

        let cache_key = RiseMessageFallbackHeightCache::make_key(
            entry,
            &current_configuration.chat,
            width,
            &current_configuration.locale,
            &current_configuration.calendar,
            &current_configuration.time_zone,
            current_configuration.layout_direction,
            display_scale,
            None,
            cx,
        );
        RiseMessageFallbackHeightCache::shared(cx).update(cx, |cache, _| cache.insert(height, &cache_key));

        let source_version = current_configuration.snapshot.source_version;
        match self.pending_runtime_height_corrections.get_mut(&entry.id) {
            Some(pending)
                if pending.entry_content_version == entry.content_version
                    && pending.source_version == source_version
                    && (pending.width - width).abs() <= 0.5 =>
            {
                if (height - pending.height).abs() <= tolerance {
                    return;
                }
                pending.height = height;
            }
            _ => {
                self.pending_runtime_height_corrections.insert(
                    entry.id.clone(),
                    RiseMessageManualListViewRuntimeHeightCorrection {
                        entry_content_version: entry.content_version,
                        source_version,
                        width,
                        cache_key,
                        height,
                    },
                );
            }
        }
        self.schedule_runtime_height_correction_flush(cx);
    }

    pub(in super::super) fn schedule_runtime_height_correction_flush(&mut self, cx: &mut Context<Self>) {
        if self.runtime_height_correction_flush_scheduled {
            return;
        }
        self.runtime_height_correction_flush_scheduled = true;
        let weak = cx.weak_entity();
        let window_handle = self.list_view.read(cx).window_handle();
        cx.defer(move |cx| {
            let _ = window_handle.update(cx, |_, window, cx| {
                let _ = weak.update(cx, |this, cx| {
                    this.runtime_height_correction_flush_scheduled = false;
                    if this.apply_runtime_height_corrections_if_possible(window, cx) {
                        return;
                    }
                    this.flush_deferred_work_if_possible(window, cx);
                });
            });
        });
    }

    pub(in super::super) fn apply_runtime_height_corrections_if_possible(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.pending_runtime_height_corrections.is_empty() || self.is_applying_snapshot {
            return false;
        }
        let Some(rendered_configuration) = self.rendered_configuration.clone() else {
            return false;
        };
        let source_version = rendered_configuration.snapshot.source_version;
        if self.requested_source_version != Some(source_version)
            || self.configuration.as_ref().map(|configuration| configuration.snapshot.source_version)
                != Some(source_version)
            || (self.rendered_preparation_width - f32::from(self.bounds.size.width)).abs() > 0.5
        {
            return false;
        }
        let is_user_scrolling = self.is_user_scrolling(cx);
        let is_scrolling_or_travelling = is_user_scrolling || self.is_travelling(cx);
        let visible_ids: HashSet<String> = self.list_view.read(cx).visible_ids().iter().cloned().collect();
        let correction_ids: HashSet<String> = self.pending_runtime_height_corrections.keys().cloned().collect();
        if !RiseMessageRuntimeHeightCorrectionPolicy::can_apply(
            self.initial_frame_ready_delivered,
            is_scrolling_or_travelling,
            &visible_ids,
            &correction_ids,
        ) {
            return false;
        }

        let corrections = std::mem::take(&mut self.pending_runtime_height_corrections);
        let display_scale = self.display_scale;
        let pixel_scale = if display_scale.is_finite() { display_scale.max(1.0) } else { 1.0 };
        let tolerance = (1.0 / pixel_scale).max(0.5);
        let mut did_change_height = false;
        let mut applied_count: i64 = 0;
        let mut total_absolute_delta: f32 = 0.0;
        let mut maximum_delta: f32 = 0.0;
        let mut corrected_visible_count: i64 = 0;
        let height_cache = RiseMessageFallbackHeightCache::shared(cx);

        for (id, correction) in &corrections {
            if correction.source_version != source_version
                || self.versions_by_id.get(id) != Some(&correction.entry_content_version)
                || (correction.width - self.rendered_preparation_width).abs() > 0.5
            {
                continue;
            }
            let current_height = self
                .measured_heights
                .get(id)
                .copied()
                .or_else(|| self.manual_items_by_id.get(id).map(|item| item.height))
                .unwrap_or(0.0);
            let absolute_delta = (correction.height - current_height).abs();
            if absolute_delta <= tolerance {
                continue;
            }
            self.measured_heights.insert(id.clone(), correction.height);
            height_cache.update(cx, |cache, _| cache.insert(correction.height, &correction.cache_key));
            did_change_height = true;
            applied_count += 1;
            total_absolute_delta += absolute_delta;
            maximum_delta = maximum_delta.max(absolute_delta);
            if visible_ids.contains(id) {
                corrected_visible_count += 1;
            }
        }
        if !did_change_height {
            return false;
        }

        let is_at_exact_bottom = self.is_at_exact_bottom(cx);
        if RiseListJumpDebug::is_enabled() {
            let message = format!(
                "HEIGHT-CORRECTION applied={applied_count} maxDelta={} visibleCorrected={corrected_visible_count} scrolling={is_user_scrolling} atBottom={is_at_exact_bottom} ids={}",
                RiseListJumpDebug::f(maximum_delta),
                corrections.keys().take(6).cloned().collect::<Vec<_>>().join(","),
            );
            RiseListJumpDebug::log(&message, cx);
        }

        ChatPerformanceTrace::shared().record_correction_batch(
            applied_count,
            total_absolute_delta,
            maximum_delta,
            corrected_visible_count,
        );

        let anchor = if self.maintains_initial_bottom_pin
            || self.initial_frame_owner.as_ref().is_some_and(|owner| owner.pins_initial_position_to_bottom)
            || (!is_user_scrolling && is_at_exact_bottom)
        {
            RiseManualListTransactionAnchor::Bottom
        } else if let Some(visible_anchor) = self.preferred_visible_anchor(cx) {
            RiseManualListTransactionAnchor::Item {
                id: visible_anchor.id,
                viewport_offset: visible_anchor.viewport_offset,
            }
        } else {
            RiseManualListTransactionAnchor::PreserveVisible
        };
        self.enqueue_rendered_items_animated(
            &rendered_configuration,
            anchor,
            false,
            Some(Self::row_slide_animation()),
            window,
            cx,
        );
        true
    }

    pub(in super::super) fn viewport_width_did_change(&mut self, width: f32, window: &mut Window, cx: &mut Context<Self>) {
        if width <= 1.0 || (self.native_preparation_width - width).abs() <= 0.5 {
            return;
        }
        if !self.initial_frame_ready_delivered {
            self.initial_viewport_readiness_generation = self.initial_viewport_readiness_generation.wrapping_add(1);
            self.initial_viewport_readiness_pass_in_flight = false;
            self.initial_viewport_readiness_watchdog = None;
            self.list_view.update(cx, |list_view, _| list_view.cancel_visible_viewport_preparation());
        }
        self.cancel_deferred_history_preparation();
        self.prepared_snapshot_finalization_generation = self.prepared_snapshot_finalization_generation.wrapping_add(1);
        self.prepared_snapshot_finalization_task = None;
        self.candidate_being_predisplayed = None;
        if self
            .pending_prepared_candidate
            .as_ref()
            .is_some_and(|candidate| (candidate.native_batch.width - width).abs() > 0.5)
        {
            self.pending_prepared_candidate = None;
        }
        self.pending_runtime_height_corrections.clear();
        self.pending_width_repreparation = true;
        cx.defer_in(window, |this, window, cx| this.flush_deferred_work_if_possible(window, cx));
    }

    pub(in super::super) fn handle_width_change_if_needed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pending_width_repreparation = false;
        let bounds_width = f32::from(self.bounds.size.width);
        if bounds_width <= 1.0 {
            return;
        }

        if self.pending_native_configuration.is_some() && self.pending_native_completion.is_some() {
            if let (Some(configuration), Some(completion)) =
                (self.pending_native_configuration.clone(), self.pending_native_completion.take())
            {
                self.prepare_native_layouts_in_range(
                    configuration,
                    self.pending_native_entry_range.clone(),
                    self.pending_native_purpose,
                    completion,
                    window,
                    cx,
                );
            }
            return;
        }
        if (self.native_preparation_width - bounds_width).abs() <= 0.5 {
            return;
        }
        let Some(configuration) = self
            .configuration
            .clone()
            .filter(|configuration| Some(configuration.snapshot.source_version) == self.requested_source_version)
        else {
            return;
        };
        if self.is_user_scrolling(cx) || self.is_applying_snapshot {
            self.pending_width_repreparation = true;
            return;
        }

        let width_retry_history_edge_load_generation = self
            .history_edge_load_generation_by_source_version
            .get(&configuration.snapshot.source_version)
            .copied();
        let prepared_configuration = configuration.clone();
        self.prepare_native_layouts(
            configuration,
            Box::new(move |native_batch, this, window, cx| {
                this.enqueue_prepared_snapshot(
                    prepared_configuration,
                    native_batch,
                    HashMap::new(),
                    None,
                    width_retry_history_edge_load_generation,
                    window,
                    cx,
                );
            }),
            window,
            cx,
        );
    }

    pub(in super::super) fn cell_configuration_version(chat: &ChatInfo) -> i64 {
        let mut hasher = DefaultHasher::new();
        chat.id.hash(&mut hasher);
        std::mem::discriminant(&chat.r#type).hash(&mut hasher);
        chat.allowed_reactions.hash(&mut hasher);
        chat.role.hash(&mut hasher);
        chat.forward_in_chat_enabled.hash(&mut hasher);
        chat.can_pin_messages.hash(&mut hasher);
        chat.can_manage_chat.hash(&mut hasher);
        hasher.finish() as i64
    }

    pub(in super::super) fn fallback_finalization_range(
        &self,
        candidate: &RiseMessageManualListViewPreparedSnapshotCandidate,
    ) -> Range<usize> {
        let entry_range = &candidate.native_batch.entry_range;
        let range = RiseFallbackMeasurementPolicy::exact_geometry_range_for_commit(
            candidate.configuration.snapshot.entries.len() as i64,
            entry_range.start as i64..entry_range.end as i64,
            self.initial_frame_ready_delivered,
            candidate.native_batch.purpose == RiseMessageManualListViewNativePreparationPurpose::DeferredHistory,
        );
        range.start as usize..range.end as usize
    }

    pub(in super::super) async fn measure_fallback_heights(
        this: WeakEntity<Self>,
        initial_candidate: RiseMessageManualListViewPreparedSnapshotCandidate,
        entry_range: Range<usize>,
        cx: &mut AsyncWindowContext,
    ) -> RiseMessageManualListViewFallbackMeasurementResult {
        let result = Self::measure_fallback_heights_pass(&this, initial_candidate, entry_range, cx).await;
        let _ = this.update_in(cx, |this, _, cx| {
            if let Some(scratch) = this.measurement_scratch_view.clone() {
                scratch.update(cx, |scratch, cx| scratch.prepare_for_manual_list_reuse(cx));
            }
        });
        result
    }

    pub(in super::super) async fn measure_fallback_heights_pass(
        this: &WeakEntity<Self>,
        mut candidate: RiseMessageManualListViewPreparedSnapshotCandidate,
        entry_range: Range<usize>,
        cx: &mut AsyncWindowContext,
    ) -> RiseMessageManualListViewFallbackMeasurementResult {
        use RiseMessageManualListViewFallbackMeasurementResult as Measurement;

        let trace_started_at = RiseClock::monotonic();
        let width = candidate.native_batch.width.max(1.0);
        let Ok(hydration) = this.update_in(cx, |this, window, cx| {
            this.hydrate_fallback_geometry(&mut candidate, entry_range, width, window, cx)
        }) else {
            return Measurement::Cancelled;
        };
        let mut hosting_measurement_count: i64 = 0;
        if hydration.ordered_unresolved.is_empty() {
            Self::record_fallback_geometry_summary(&candidate, &hydration, 0, 0, trace_started_at);
            return Measurement::Completed(candidate);
        }

        let mut previous_active_measurement_at: Option<f64> = None;
        'measurement_state_loop: loop {
            let Ok((is_applying, is_scrolling)) = this.update_in(cx, |this, _, cx| this.fallback_measurement_state(cx))
            else {
                return Measurement::Cancelled;
            };
            if is_applying {
                let estimates = Self::remaining_unresolved_count(&candidate, &hydration);
                Self::record_fallback_geometry_summary(&candidate, &hydration, hosting_measurement_count, estimates, trace_started_at);
                return Measurement::Paused(candidate);
            }

            if is_scrolling {
                let Ok((holds_fast_fling, _)) = this.update_in(cx, |this, _, cx| this.fallback_measurement_pacing(cx))
                else {
                    return Measurement::Cancelled;
                };
                if holds_fast_fling {
                    let estimates = Self::remaining_unresolved_count(&candidate, &hydration);
                    Self::record_fallback_geometry_summary(&candidate, &hydration, hosting_measurement_count, estimates, trace_started_at);
                    return Measurement::Paused(candidate);
                }
                for &index in &hydration.critical_unresolved {
                    if Self::has_exact_fallback_height_at(&candidate, index) {
                        continue;
                    }
                    let Ok((is_applying, is_scrolling)) =
                        this.update_in(cx, |this, _, cx| this.fallback_measurement_state(cx))
                    else {
                        return Measurement::Cancelled;
                    };
                    if is_applying {
                        let estimates = Self::remaining_unresolved_count(&candidate, &hydration);
                        Self::record_fallback_geometry_summary(&candidate, &hydration, hosting_measurement_count, estimates, trace_started_at);
                        return Measurement::Paused(candidate);
                    }
                    if !is_scrolling {
                        break;
                    }
                    Self::wait_before_next_fallback_measurement(previous_active_measurement_at, cx).await;
                    let Ok((is_applying, is_scrolling)) =
                        this.update_in(cx, |this, _, cx| this.fallback_measurement_state(cx))
                    else {
                        return Measurement::Cancelled;
                    };
                    if is_applying {
                        let estimates = Self::remaining_unresolved_count(&candidate, &hydration);
                        Self::record_fallback_geometry_summary(&candidate, &hydration, hosting_measurement_count, estimates, trace_started_at);
                        return Measurement::Paused(candidate);
                    }
                    if !is_scrolling {
                        break;
                    }
                    let Ok((holds_fast_fling, _)) =
                        this.update_in(cx, |this, _, cx| this.fallback_measurement_pacing(cx))
                    else {
                        return Measurement::Cancelled;
                    };
                    if holds_fast_fling {
                        let estimates = Self::remaining_unresolved_count(&candidate, &hydration);
                        Self::record_fallback_geometry_summary(&candidate, &hydration, hosting_measurement_count, estimates, trace_started_at);
                        return Measurement::Paused(candidate);
                    }
                    previous_active_measurement_at = Some(RiseClock::monotonic().as_secs_f64());
                    if Self::measure_hosted_fallback_batch(this, &mut candidate, &[index], 0.0, width, cx).await.is_none() {
                        return Measurement::Cancelled;
                    }
                    hosting_measurement_count += 1;
                }

                let Ok((is_applying, is_scrolling)) =
                    this.update_in(cx, |this, _, cx| this.fallback_measurement_state(cx))
                else {
                    return Measurement::Cancelled;
                };
                if is_scrolling || is_applying {
                    let estimates = Self::remaining_unresolved_count(&candidate, &hydration);
                    Self::record_fallback_geometry_summary(&candidate, &hydration, hosting_measurement_count, estimates, trace_started_at);
                    return Measurement::Paused(candidate);
                }
                continue 'measurement_state_loop;
            }

            let ordered_unresolved = &hydration.ordered_unresolved;
            let mut cursor = 0;
            while cursor < ordered_unresolved.len() {
                if Self::has_exact_fallback_height_at(&candidate, ordered_unresolved[cursor]) {
                    cursor += 1;
                    continue;
                }
                let Ok((is_applying, is_scrolling)) =
                    this.update_in(cx, |this, _, cx| this.fallback_measurement_state(cx))
                else {
                    return Measurement::Cancelled;
                };
                if is_applying {
                    let estimates = Self::remaining_unresolved_count(&candidate, &hydration);
                    Self::record_fallback_geometry_summary(&candidate, &hydration, hosting_measurement_count, estimates, trace_started_at);
                    return Measurement::Paused(candidate);
                }
                if is_scrolling {
                    continue 'measurement_state_loop;
                }
                let Ok((_, initial_frame_ready_delivered)) =
                    this.update_in(cx, |this, _, cx| this.fallback_measurement_pacing(cx))
                else {
                    return Measurement::Cancelled;
                };
                let Ok(jump_pending) =
                    this.read_with(cx, |this, _| this.pending_scroll_reset || this.pending_scroll_request.is_some())
                else {
                    return Measurement::Cancelled;
                };
                let paced = initial_frame_ready_delivered && !jump_pending;
                if paced {
                    Self::wait_before_next_fallback_measurement(previous_active_measurement_at, cx).await;
                    let Ok((is_applying, is_scrolling)) =
                        this.update_in(cx, |this, _, cx| this.fallback_measurement_state(cx))
                    else {
                        return Measurement::Cancelled;
                    };
                    if is_applying {
                        let estimates = Self::remaining_unresolved_count(&candidate, &hydration);
                        Self::record_fallback_geometry_summary(&candidate, &hydration, hosting_measurement_count, estimates, trace_started_at);
                        return Measurement::Paused(candidate);
                    }
                    if is_scrolling {
                        continue 'measurement_state_loop;
                    }
                }
                let batch_positions: Vec<usize> = (cursor..ordered_unresolved.len())
                    .filter(|&position| !Self::has_exact_fallback_height_at(&candidate, ordered_unresolved[position]))
                    .take(Self::HOSTED_MEASUREMENT_BATCH_LIMIT)
                    .collect();
                let batch: Vec<usize> = batch_positions.iter().map(|&position| ordered_unresolved[position]).collect();
                let budget_seconds =
                    if paced { Self::PACED_HOSTED_MEASUREMENT_BUDGET } else { Self::UNPACED_HOSTED_MEASUREMENT_BUDGET };
                previous_active_measurement_at = Some(RiseClock::monotonic().as_secs_f64());
                let Some(measured) =
                    Self::measure_hosted_fallback_batch(this, &mut candidate, &batch, budget_seconds, width, cx).await
                else {
                    return Measurement::Cancelled;
                };
                hosting_measurement_count += measured as i64;
                cursor = batch_positions.get(measured.max(1) - 1).map_or(ordered_unresolved.len(), |&position| position + 1);
            }
            break 'measurement_state_loop;
        }

        Self::record_fallback_geometry_summary(&candidate, &hydration, hosting_measurement_count, 0, trace_started_at);
        if this.read_with(cx, |this, _| !this.initial_frame_ready_delivered).unwrap_or(false) {
            let (native, exact, cache) = (hydration.native_count, hydration.exact_count, hydration.cache_hit_count);
            let ms = RiseClock::monotonic().saturating_sub(trace_started_at).as_millis();
            RiseColdOpenTrace::mark(|| {
                format!("list measure native={native} exact={exact} cache={cache} hosted={hosting_measurement_count} ms={ms}")
            });
        }
        Measurement::Completed(candidate)
    }

    pub(in super::super) fn fallback_measurement_state(&self, cx: &App) -> (bool, bool) {
        (self.is_applying_snapshot, self.is_user_scrolling(cx))
    }

    /// no hosted measuring at fling speed and none spaced by frames before the first one
    /// the cold chat has no list on screen to keep smooth yet
    pub(in super::super) fn fallback_measurement_pacing(&self, cx: &App) -> (bool, bool) {
        (self.holds_edge_page_for_fast_fling(cx), self.initial_frame_ready_delivered)
    }

    pub(in super::super) async fn wait_before_next_fallback_measurement(
        previous_measurement_at: Option<f64>,
        cx: &mut AsyncWindowContext,
    ) {
        let elapsed = previous_measurement_at.map(|previous| RiseClock::monotonic().as_secs_f64() - previous);
        let delay = RiseMessageActiveScrollFallbackMeasurementPolicy::delay_before_next_measurement(
            elapsed,
            RiseMessageActiveScrollFallbackMeasurementPolicy::actor_interval(60),
        );
        if delay > 0.0 {
            cx.background_executor().timer(Duration::from_secs_f64(delay)).await;
        }
    }

    pub(in super::super) fn has_exact_fallback_height_at(
        candidate: &RiseMessageManualListViewPreparedSnapshotCandidate,
        index: usize,
    ) -> bool {
        let id = &candidate.configuration.snapshot.entries[index].id;
        RiseMessagePreparedGeometryPolicy::has_exact_fallback_height(candidate.fallback_heights.get(id).copied())
    }

    pub(in super::super) fn remaining_unresolved_count(
        candidate: &RiseMessageManualListViewPreparedSnapshotCandidate,
        hydration: &RiseMessageManualListViewFallbackHydration,
    ) -> i64 {
        hydration.unresolved_indices.iter().filter(|&&index| !Self::has_exact_fallback_height_at(candidate, index)).count()
            as i64
    }

    pub(in super::super) fn record_fallback_geometry_summary(
        candidate: &RiseMessageManualListViewPreparedSnapshotCandidate,
        hydration: &RiseMessageManualListViewFallbackHydration,
        hosted_measurements: i64,
        estimates: i64,
        trace_started_at: Duration,
    ) {
        ChatPerformanceTrace::shared().record_geometry_summary(
            Self::preparation_purpose_trace_name(candidate.native_batch.purpose),
            hydration.requested_range.start as i64..hydration.requested_range.end as i64,
            hydration.native_count,
            hydration.exact_count,
            hydration.cache_hit_count,
            hosted_measurements,
            estimates,
            (RiseClock::monotonic().saturating_sub(trace_started_at)).as_secs_f64() * 1_000.0,
        );
    }

    pub(in super::super) fn hydrate_fallback_geometry(
        &mut self,
        candidate: &mut RiseMessageManualListViewPreparedSnapshotCandidate,
        entry_range: Range<usize>,
        width: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> RiseMessageManualListViewFallbackHydration {
        let configuration = candidate.configuration.clone();
        let display_scale = self.display_scale;
        let height_cache = RiseMessageFallbackHeightCache::shared(cx);
        let mut geometry_context = RiseMessageFallbackGeometryContext::new(&configuration.chat, cx);
        let entries = &configuration.snapshot.entries;
        let lower_bound = entries.len().min(entry_range.start);
        let upper_bound = lower_bound.max(entries.len().min(entry_range.end));
        let requested_range = lower_bound..upper_bound;
        let priority_range = self.fallback_measurement_priority_range(candidate, requested_range.clone(), window, cx);

        let mut native_count = 0;
        let mut exact_count = 0;
        let mut cache_hit_count = 0;
        let mut unresolved_indices: Vec<usize> = Vec::with_capacity(requested_range.len());

        for index in requested_range.clone() {
            let entry = &entries[index];
            if entry.kind != ChatMessageRenderEntryKind::Message {
                continue;
            }
            if Self::has_prepared_native_geometry(index, candidate) {
                native_count += 1;
                continue;
            }
            if RiseMessagePreparedGeometryPolicy::has_exact_fallback_height(
                candidate.fallback_heights.get(&entry.id).copied(),
            ) {
                continue;
            }
            candidate.fallback_heights.remove(&entry.id);

            let exact_height = RiseExactFallbackGeometry::row_height(entry, &configuration, width, cx);
            let cache_key = RiseMessageFallbackHeightCache::make_key(
                entry,
                &configuration.chat,
                width,
                &configuration.locale,
                &configuration.calendar,
                &configuration.time_zone,
                configuration.layout_direction,
                display_scale,
                Some(&mut geometry_context),
                cx,
            );
            if let Some(cached_height) = height_cache.update(cx, |cache, _| cache.height(&cache_key)) {
                candidate.fallback_heights.insert(entry.id.clone(), cached_height);
                cache_hit_count += 1;
                continue;
            }
            let native = NativeBubbleRegistry::kind_for(entry, &configuration, width, cx);
            if let Some(native) = native {
                NativeBubbleRegistry::report_row(native.kind.name, entry, &configuration, width, native.height, cx);
            }
            if let Some(exact_height) = native.map(|native| native.height).or(exact_height) {
                candidate.fallback_heights.insert(entry.id.clone(), exact_height);
                exact_count += 1;
                height_cache.update(cx, |cache, _| cache.insert(exact_height, &cache_key));
                continue;
            }
            unresolved_indices.push(index);
        }

        let unresolved_set: HashSet<usize> = unresolved_indices.iter().copied().collect();
        let critical_indices: Vec<i64> =
            self.scroll_safe_critical_geometry_indices(candidate).into_iter().map(|index| index as i64).collect();
        let unresolved_i64: Vec<i64> = unresolved_indices.iter().map(|index| *index as i64).collect();
        let critical_unresolved: Vec<usize> = RiseMessageActiveScrollFallbackMeasurementPolicy::unresolved_critical_indices(
            &critical_indices,
            &unresolved_i64,
        )
        .into_iter()
        .map(|index| index as usize)
        .collect();
        let mut visited: HashSet<usize> = HashSet::with_capacity(unresolved_indices.len());
        let mut ordered_unresolved: Vec<usize> = Vec::with_capacity(unresolved_indices.len());
        for &index in &critical_unresolved {
            ordered_unresolved.push(index);
            visited.insert(index);
        }
        for index in priority_range {
            if unresolved_set.contains(&index) && visited.insert(index) {
                ordered_unresolved.push(index);
            }
        }
        for &index in &unresolved_indices {
            if !visited.contains(&index) {
                ordered_unresolved.push(index);
            }
        }

        RiseMessageManualListViewFallbackHydration {
            requested_range,
            native_count,
            exact_count,
            cache_hit_count,
            unresolved_indices,
            critical_unresolved,
            ordered_unresolved,
        }
    }

    pub(in super::super) const HOSTED_MEASUREMENT_BATCH_LIMIT: usize = 64;
    pub(in super::super) const PACED_HOSTED_MEASUREMENT_BUDGET: f64 = 0.004;
    pub(in super::super) const UNPACED_HOSTED_MEASUREMENT_BUDGET: f64 = 0.030;

    /// measures indices in order inside one frame until budget_seconds is spent (at least one row)
    /// returns how many rows were measured, none when the list is gone
    pub(in super::super) async fn measure_hosted_fallback_batch(
        this: &WeakEntity<Self>,
        candidate: &mut RiseMessageManualListViewPreparedSnapshotCandidate,
        indices: &[usize],
        budget_seconds: f64,
        width: f32,
        cx: &mut AsyncWindowContext,
    ) -> Option<usize> {
        if indices.is_empty() {
            return Some(0);
        }
        let entries: Vec<Arc<ChatMessageRenderEntry>> =
            indices.iter().map(|&index| candidate.configuration.snapshot.entries[index].clone()).collect();
        let receiver = this
            .update_in(cx, |this, _, cx| {
                this.request_hosted_fallback_measurement(&entries, candidate, width, budget_seconds, cx)
            })
            .ok()?;
        let measured_heights = receiver.await.ok()?;
        let configuration = candidate.configuration.clone();
        for (entry, &measured_height) in entries.iter().zip(measured_heights.iter()) {
            candidate.fallback_heights.insert(entry.id.clone(), measured_height);
        }
        this.update_in(cx, |this, _, cx| {
            for (entry, &measured_height) in entries.iter().zip(measured_heights.iter()) {
                let cache_key = RiseMessageFallbackHeightCache::make_key(
                    entry,
                    &configuration.chat,
                    width,
                    &configuration.locale,
                    &configuration.calendar,
                    &configuration.time_zone,
                    configuration.layout_direction,
                    this.display_scale,
                    None,
                    cx,
                );
                RiseMessageFallbackHeightCache::shared(cx).update(cx, |cache, _| cache.insert(measured_height, &cache_key));
            }
        })
        .ok()?;
        Some(measured_heights.len())
    }

    pub(in super::super) fn request_hosted_fallback_measurement(
        &mut self,
        entries: &[Arc<ChatMessageRenderEntry>],
        candidate: &RiseMessageManualListViewPreparedSnapshotCandidate,
        width: f32,
        budget_seconds: f64,
        cx: &mut Context<Self>,
    ) -> oneshot::Receiver<Vec<f32>> {
        let view = match self.measurement_scratch_view.clone() {
            Some(existing) => existing,
            None => {
                let created = cx.new(RiseManualMessageHostingView::new);
                self.measurement_scratch_view = Some(created.clone());
                created
            }
        };
        let rows = entries
            .iter()
            .map(|entry| (entry.clone(), candidate.native_batch.hosted_text.get(&entry.id).cloned()))
            .collect();
        let (sender, receiver) = oneshot::channel();
        self.pending_fallback_measurement = Some(RiseMessageManualListViewFallbackMeasurementRequest {
            view,
            width,
            configuration: candidate.configuration.clone(),
            rows,
            budget_seconds,
            sender,
        });
        cx.notify();
        receiver
    }

    pub(in super::super) fn fallback_measurement_priority_range(
        &self,
        candidate: &RiseMessageManualListViewPreparedSnapshotCandidate,
        requested_range: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Range<usize> {
        if requested_range.is_empty() {
            return requested_range;
        }
        let snapshot = &candidate.configuration.snapshot;
        if self.did_restore_initial_position && !self.pending_scroll_reset && self.pending_scroll_request.is_none() {
            let visible_indices: Vec<usize> = self
                .list_view
                .read(cx)
                .visible_ids()
                .iter()
                .filter_map(|id| snapshot.indices_by_id.get(id).copied())
                .collect();
            if let (Some(first_visible), Some(last_visible)) =
                (visible_indices.iter().min().copied(), visible_indices.iter().max().copied())
            {
                let guarded = RiseFallbackMeasurementPolicy::guarded_visible_range(
                    first_visible as i64..last_visible as i64 + 1,
                    snapshot.entries.len() as i64,
                );
                let lower_bound = requested_range.start.max(guarded.start as usize);
                let upper_bound = requested_range.end.min(guarded.end as usize);
                if lower_bound < upper_bound {
                    return lower_bound..upper_bound;
                }
            }
        }
        let centered_at = self
            .initial_predisplay_center_index(candidate, window, cx)
            .unwrap_or(requested_range.end - 1);
        let nearby = RiseFallbackMeasurementPolicy::nearby_range(
            requested_range.start as i64..requested_range.end as i64,
            centered_at as i64,
        );
        nearby.start as usize..nearby.end as usize
    }

    pub(in super::super) fn unresolved_fallback_geometry_count(
        &self,
        candidate: &RiseMessageManualListViewPreparedSnapshotCandidate,
        entry_range: Range<usize>,
    ) -> i64 {
        let reusing_retained_item_ids = HashSet::new();
        let entry_count = candidate.configuration.snapshot.entries.len();
        let lower_bound = entry_count.min(entry_range.start);
        let upper_bound = lower_bound.max(entry_count.min(entry_range.end));
        (lower_bound..upper_bound)
            .filter(|&index| self.fallback_geometry_is_unresolved(candidate, index, &reusing_retained_item_ids))
            .count() as i64
    }

    pub(in super::super) fn fallback_geometry_is_unresolved(
        &self,
        candidate: &RiseMessageManualListViewPreparedSnapshotCandidate,
        index: usize,
        reusing_retained_item_ids: &HashSet<String>,
    ) -> bool {
        let entry = &candidate.configuration.snapshot.entries[index];
        if entry.kind != ChatMessageRenderEntryKind::Message
            || Self::has_prepared_native_geometry(index, candidate)
            || RiseMessagePreparedGeometryPolicy::has_exact_fallback_height(
                candidate.fallback_heights.get(&entry.id).copied(),
            )
        {
            return false;
        }
        !(reusing_retained_item_ids.contains(&entry.id)
            && RiseMessagePreparedGeometryPolicy::can_reuse_retained_item_geometry(
                self.versions_by_id.get(&entry.id).copied(),
                entry.content_version,
                self.manual_items_by_id.get(&entry.id).map(|item| item.height),
            ))
    }

    pub(in super::super) fn can_reuse_measured_fallback_heights(
        &self,
        next: &RiseMessageCollectionConfiguration,
        width: f32,
    ) -> bool {
        if (self.measured_height_width - width).abs() > 0.5 {
            return false;
        }
        let Some(current) = self.rendered_configuration.as_ref() else {
            return false;
        };
        current.chat_id == next.chat_id
            && current.chat.r#type == next.chat.r#type
            && current.layout_direction == next.layout_direction
            && current.locale.identifier() == next.locale.identifier()
            && current.calendar.identifier == next.calendar.identifier
            && current.time_zone.identifier() == next.time_zone.identifier()
    }

    pub(in super::super) fn prepare_native_layouts_in_range(
        &mut self,
        configuration: Rc<RiseMessageCollectionConfiguration>,
        requested_entry_range: Option<Range<usize>>,
        purpose: RiseMessageManualListViewNativePreparationPurpose,
        completion: RiseMessageManualListViewNativeCompletion,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let preparation_width = f32::from(self.bounds.size.width);
        if preparation_width <= 1.0 {
            self.pending_native_configuration = Some(configuration);
            self.pending_native_completion = Some(completion);
            self.pending_native_entry_range = requested_entry_range;
            self.pending_native_purpose = purpose;
            return;
        }
        self.pending_native_configuration = None;
        self.pending_native_completion = None;
        self.pending_native_entry_range = None;
        self.pending_native_purpose = RiseMessageManualListViewNativePreparationPurpose::Foreground;
        self.native_preparation_task = None;
        self.native_preparation_generation = self.native_preparation_generation.wrapping_add(1);
        let generation = self.native_preparation_generation;
        let preparation_revision = self.configuration_revision;
        self.native_preparation_width = preparation_width;
        let expected_versions = configuration.snapshot.versions_by_id.clone();
        let entry_count = configuration.snapshot.entries.len();
        let proposed_range =
            requested_entry_range.unwrap_or_else(|| self.priority_preparation_range(&configuration));
        let entry_lower_bound = entry_count.min(proposed_range.start);
        let entry_upper_bound = entry_lower_bound.max(entry_count.min(proposed_range.end));
        let entry_range = entry_lower_bound..entry_upper_bound;
        let purpose_name = Self::preparation_purpose_trace_name(purpose);
        let trace_token = ChatPerformanceTrace::shared().begin_operation(
            "native.prepare.plan",
            HashMap::from([
                ("purpose".to_owned(), purpose_name.to_owned()),
                ("range".to_owned(), format!("{}..<{}", entry_range.start, entry_range.end)),
            ]),
        );

        let mut staged_layouts: HashMap<String, Arc<AsyncTextBubblePreparedLayout>> = HashMap::new();
        let mut staged_versions: HashMap<String, i64> = HashMap::new();
        let mut staged_ownership: HashMap<String, bool> = HashMap::new();
        let mut eligible_ownership: HashMap<String, bool> = HashMap::new();
        let mut requests: Vec<RiseMessageManualListViewNativePreparationRequest> =
            Vec::with_capacity(entry_range.len());
        let mut hosted_requests: Vec<(String, FormattedTextRenderSeed)> = Vec::new();
        let mut shared_layout_cache_hits = 0;
        let layout_cache = AsyncTextBubblePreparedLayoutCache::shared();
        let time_zone_identifier = configuration.time_zone.identifier().to_owned();
        let theme = configuration.theme_store.read(cx).current_theme().clone();

        for entry in &configuration.snapshot.entries[entry_range.clone()] {
            if entry.kind != ChatMessageRenderEntryKind::Message {
                continue;
            }
            let (Some(message), Some(group)) = (entry.message.as_ref(), entry.group) else {
                continue;
            };
            let is_own = MessageList::is_own_bubble(message, &configuration.chat, cx);
            let allows_incoming_group_without_chrome =
                configuration.chat.r#type == ChatType::Group && !is_own && !group.is_first;
            let maximum_width = MessageSizes::available_bubble_width(preparation_width, &configuration.chat, is_own);
            let spec = RiseAsyncTextMessageEligibility::layout_spec(maximum_width, preparation_width, Some(is_own));

            if let Some(current) = self.native_layouts.get(&entry.id)
                && RiseMessageNativeLayoutReusePolicy::can_reuse(
                    entry.content_version,
                    self.native_layout_versions.get(&entry.id).copied(),
                    &current.spec,
                    &spec,
                    &current.seed.time_zone_identifier,
                    &time_zone_identifier,
                    self.native_ownership_by_id.get(&entry.id).copied(),
                    is_own,
                )
            {
                staged_layouts.insert(entry.id.clone(), current.clone());
                staged_versions.insert(entry.id.clone(), entry.content_version);
                staged_ownership.insert(entry.id.clone(), is_own);
                layout_cache.insert(current.clone());
                continue;
            }

            let Some(seed) = RiseAsyncTextMessageEligibility::seed(
                message,
                &configuration.chat,
                is_own,
                allows_incoming_group_without_chrome,
                Some(time_zone_identifier.clone()),
                entry.search_highlight_ranges.clone().unwrap_or_default(),
                entry.plain_text_eligibility,
            ) else {
                if MessageContent::bubble_kind(message, Some(configuration.chat.as_ref())) == MessageContentKind::Text
                    && message.document_raw.is_none()
                    && message.resolved_entities().custom_emojis.is_empty()
                    && !RichTextMarkup::has_code_block(&message.text)
                {
                    let highlight_words = entry
                        .search_highlight_ranges
                        .as_ref()
                        .map(|ranges| ChatMessageSearchHighlight::words(&message.text, ranges))
                        .unwrap_or_default();
                    hosted_requests.push((
                        entry.id.clone(),
                        FormattedTextRenderSeed::new(
                            message.text.clone(),
                            MessageContent::formatted_text_style(&theme, is_own),
                            FormattedTextOptions { highlight_words, ..Default::default() },
                            RoleMentionComposer::render_spans(
                                &message.resolved_entities().role_mentions,
                                &message.chat_id,
                                is_own,
                                cx,
                            )
                            .into_iter()
                            .chain(TopicMentionComposer::render_spans(
                                &message.resolved_entities().topic_mentions,
                                &message.chat_id,
                            ))
                            .chain(MessageContent::text_link_spans(&message.resolved_entities().text_links))
                            .collect(),
                        ),
                    ));
                }
                continue;
            };

            if let Some(cached) = layout_cache.layout(&seed, &spec) {
                staged_layouts.insert(entry.id.clone(), cached);
                staged_versions.insert(entry.id.clone(), entry.content_version);
                staged_ownership.insert(entry.id.clone(), is_own);
                shared_layout_cache_hits += 1;
                continue;
            }

            if let Some(reused) = self
                .native_layouts
                .get(&entry.id)
                .filter(|current| current.spec == spec)
                .and_then(|current| current.reusing_geometry(&seed))
            {
                let reused = Arc::new(reused);
                staged_layouts.insert(entry.id.clone(), reused.clone());
                staged_versions.insert(entry.id.clone(), entry.content_version);
                staged_ownership.insert(entry.id.clone(), is_own);
                layout_cache.insert(reused);
                continue;
            }
            eligible_ownership.insert(entry.id.clone(), is_own);
            requests.push(RiseMessageManualListViewNativePreparationRequest {
                id: entry.id.clone(),
                content_version: entry.content_version,
                seed,
                spec,
            });
        }

        ChatPerformanceTrace::shared().end_operation(
            trace_token,
            0.0,
            HashMap::from([
                ("requests".to_owned(), requests.len().to_string()),
                ("reused".to_owned(), staged_layouts.len().to_string()),
                ("shared_cache_hits".to_owned(), shared_layout_cache_hits.to_string()),
            ]),
        );

        if requests.is_empty() && hosted_requests.is_empty() {
            self.native_preparation_task = None;
            let detail = format!(
                "purpose={purpose_name} range={}..<{} native={} prepared=0",
                entry_range.start,
                entry_range.end,
                staged_layouts.len(),
            );
            MessagePaginationTrace::record(
                "render.geometry",
                &configuration.chat_id,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(entry_count as i64),
                None,
                Some(&detail),
                cx,
            );
            completion(
                RiseMessageManualListViewPreparedNativeBatch {
                    chat_id: configuration.chat_id.clone(),
                    source_version: configuration.snapshot.source_version,
                    configuration_revision: preparation_revision,
                    width: preparation_width,
                    layouts: staged_layouts,
                    versions: staged_versions,
                    ownership_by_id: staged_ownership,
                    entry_range,
                    purpose,
                    hosted_text: HashMap::new(),
                },
                self,
                window,
                cx,
            );
            return;
        }

        let layout_cache_epoch = layout_cache.epoch();
        let text_system = cx.text_system().clone();
        self.native_preparation_task = Some(cx.spawn_in(window, async move |this, cx| {
            let cancelled = Arc::new(AtomicBool::new(false));
            let _cancellation = RiseMessageManualListViewWorkerCancellation(cancelled.clone());
            let (prepared, prepared_text, worker_duration_ms) = cx
                .background_executor()
                .spawn(async move {
                    let worker_started_at = RiseClock::monotonic();
                    let mut result: Vec<(String, i64, Arc<AsyncTextBubblePreparedLayout>)> =
                        Vec::with_capacity(requests.len());
                    for request in requests {
                        if cancelled.load(Ordering::Relaxed) {
                            break;
                        }
                        let layout = Arc::new(AsyncTextBubblePreparer::prepare_synchronously(
                            request.seed,
                            request.spec,
                            &text_system,
                        ));
                        AsyncTextBubblePreparedLayoutCache::shared().insert_at_epoch(layout.clone(), layout_cache_epoch);
                        result.push((request.id, request.content_version, layout));
                    }
                    let mut text: HashMap<String, FormattedTextInlinePreparation> =
                        HashMap::with_capacity(hosted_requests.len());
                    for (id, seed) in &hosted_requests {
                        if cancelled.load(Ordering::Relaxed) {
                            break;
                        }
                        text.insert(id.clone(), FormattedText::prepare(seed));
                    }
                    (result, text, RiseClock::monotonic().saturating_sub(worker_started_at).as_secs_f64() * 1_000.0)
                })
                .await;

            let _ = this.update_in(cx, |this, window, cx| {
                if !this.initial_frame_ready_delivered {
                    RiseColdOpenTrace::mark(|| {
                        format!(
                            "list layouts worker ms={} layouts={} text={} cancelled=false",
                            worker_duration_ms as i64,
                            prepared.len(),
                            prepared_text.len()
                        )
                    });
                }
                if this.native_preparation_generation != generation
                    || this.configuration_revision != preparation_revision
                    || this.requested_source_version != Some(configuration.snapshot.source_version)
                    || this.configuration.as_ref().map(|current| current.chat_id.as_str())
                        != Some(configuration.chat_id.as_str())
                {
                    return;
                }

                ChatPerformanceTrace::shared().record(
                    "native.prepare.worker",
                    HashMap::from([
                        (rise_otp_key::DURATION_MS.to_owned(), format!("{worker_duration_ms:.3}")),
                        ("prepared".to_owned(), prepared.len().to_string()),
                        ("purpose".to_owned(), purpose_name.to_owned()),
                    ]),
                );
                let install_token = ChatPerformanceTrace::shared().begin_operation(
                    "native.prepare.install",
                    HashMap::from([("prepared".to_owned(), prepared.len().to_string())]),
                );
                let prepared_count = prepared.len();
                let mut completed_layouts = staged_layouts;
                let mut completed_versions = staged_versions;
                let mut completed_ownership = staged_ownership;
                for (id, content_version, layout) in prepared {
                    if expected_versions.get(&id) != Some(&content_version) {
                        continue;
                    }
                    if let Some(is_own) = eligible_ownership.get(&id).copied() {
                        completed_ownership.insert(id.clone(), is_own);
                    }
                    completed_versions.insert(id.clone(), content_version);
                    completed_layouts.insert(id, layout);
                }
                this.native_preparation_task = None;
                let detail = format!(
                    "purpose={purpose_name} range={}..<{} native={} prepared={prepared_count}",
                    entry_range.start,
                    entry_range.end,
                    completed_layouts.len(),
                );
                MessagePaginationTrace::record(
                    "render.geometry",
                    &configuration.chat_id,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some(entry_count as i64),
                    None,
                    Some(&detail),
                    cx,
                );
                ChatPerformanceTrace::shared().end_operation(
                    install_token,
                    0.0,
                    HashMap::from([("result".to_owned(), completed_layouts.len().to_string())]),
                );
                completion(
                    RiseMessageManualListViewPreparedNativeBatch {
                        chat_id: configuration.chat_id.clone(),
                        source_version: configuration.snapshot.source_version,
                        configuration_revision: preparation_revision,
                        width: preparation_width,
                        layouts: completed_layouts,
                        versions: completed_versions,
                        ownership_by_id: completed_ownership,
                        entry_range,
                        purpose,
                        hosted_text: prepared_text,
                    },
                    this,
                    window,
                    cx,
                );
            });
        }));
    }
}
