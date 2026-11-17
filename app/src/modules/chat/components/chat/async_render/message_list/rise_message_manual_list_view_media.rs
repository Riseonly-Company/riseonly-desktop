use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::time::Duration;
use gpui::{App, Context, Window};
use rise_core::rise_app_events::RiseAppEvents;
use rise_ui::animated_media_playback_scope::ANIMATED_MEDIA_PLAYBACK_SCOPE_DID_CHANGE;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media::video_warm_start_prefetch::VideoWarmStartPrefetch;
use crate::modules::chat::components::chat::async_render::chat_message_render_snapshot::{ChatMessageRenderEntry, ChatMessageRenderEntryKind};
use crate::modules::chat::components::chat::async_render::chat_performance_trace::ChatPerformanceTrace;
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_types::{RiseManualListScrollMetrics, RiseManualListTransactionAnchor};
use crate::modules::chat::components::chat::async_render::rise_native_text_message_cell::RiseNativeTextMessageCell;
use crate::modules::chat::components::chat::bubble::message_content::{MessageContent, MessageContentKind};
use crate::modules::chat::components::chat::message_list::MessageList;
use crate::modules::chat::components::chat::async_render::async_text_bubble_predisplay::{AsyncTextBubblePredisplayCache, AsyncTextBubblePredisplayRequest};
use crate::modules::chat::components::chat::video_note::video_note_message_view::VideoNoteMessageView;
use crate::modules::chat::components::chat::video_note::video_note_player_store::VideoNotePlayerStore;
use crate::core::ui::clever_image_disk_cache::{CleverImageDiskPrefetch, CleverImagePrefetchItem};
use crate::core::ui::lottie_sticker::lottie_sticker_prefetch::LottieStickerPrefetch;
use crate::modules::post::components::post::post_media_cache_policy::PostMediaCachePolicy;
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::chat::stores::message::message_actions::message_types::{MessageMediaItem, MessageMediaKind, MessageStickerKind, RemoteMediaURL};
use super::super::rise_message_list_view::*;

impl RiseMessageManualListView {
    pub fn set_animated_media_playback_active(
        &mut self,
        is_active: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.requested_animated_media_playback_active == is_active {
            return;
        }
        self.requested_animated_media_playback_active = is_active;
        self.update_meaningful_activity(cx);
        self.update_animated_media_playback_scope();
        self.refresh_speculative_preparation(window, cx);
        if !is_active {
            self.cancel_directional_media_prefetch(cx);
            self.cancel_remote_page_prefetch(window, cx);
        } else if self.initial_frame_ready_delivered {
            self.last_reported_mark_candidate_ids.clear();
            let visible_ids = self.visible_ids(cx);
            self.report_visible_mark_candidates(&visible_ids, window, cx);
            self.prefetch_directional_media(&visible_ids, cx);
        }
    }

    pub(in super::super) fn cancel_remote_page_prefetch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(on_prefetch_page) = self
            .configuration
            .as_ref()
            .and_then(|configuration| configuration.on_prefetch_page.clone())
        {
            on_prefetch_page(None, window, cx);
        }
        self.remote_prefetch_direction = None;
        self.remote_prefetch_source_version = None;
    }

    pub(in super::super) fn update_animated_media_playback_for_scroll(&mut self, metrics: &RiseManualListScrollMetrics, cx: &mut App) {
        let should_suspend = RiseMessageAnimatedMediaScrollPolicy::should_suspend_playback(
            self.suspends_animated_media_for_fast_scroll,
            metrics,
        );
        if self.suspends_animated_media_for_fast_scroll == should_suspend {
            return;
        }
        self.suspends_animated_media_for_fast_scroll = should_suspend;
        ChatPerformanceTrace::shared().record(
            "animated_media.fast_scroll",
            HashMap::from([
                ("suspended".to_owned(), if should_suspend { "1" } else { "0" }.to_owned()),
                ("velocity".to_owned(), (f32::from(metrics.velocity_y).abs().round() as i64).to_string()),
            ]),
        );
        self.last_media_prefetch_center = None;
        if should_suspend {
            self.media_prefetch_generation = self.media_prefetch_generation.wrapping_add(1);
            VideoWarmStartPrefetch::cancel_directional(&self.media_prefetch_scope, self.media_prefetch_generation, cx);
        }
        self.update_animated_media_playback_scope();
    }

    pub(in super::super) fn update_animated_media_playback_scope(&mut self) {
        let is_active =
            self.requested_animated_media_playback_active && !self.suspends_animated_media_for_fast_scroll;
        if self.is_animated_media_playback_active == is_active {
            return;
        }
        self.is_animated_media_playback_active = is_active;
        RiseAppEvents::default_center().post(ANIMATED_MEDIA_PLAYBACK_SCOPE_DID_CHANGE);
    }

    pub(in super::super) fn observe_video_note_playback(&self, window: &mut Window, cx: &mut Context<Self>) {
        let store = VideoNotePlayerStore::shared(cx);
        cx.observe_in(&store, window, |this, _, window, cx| {
            this.video_note_playback_did_change(window, cx);
        })
        .detach();
    }

    /// grows the playing video note row in place and shrinks the previous one like telegram
    pub(in super::super) fn video_note_playback_did_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let playing_id = Self::playing_video_note_message_id(cx);
        if playing_id == self.grown_video_note_message_id {
            return;
        }
        let Some(rendered_configuration) = self.rendered_configuration.clone() else {
            self.grown_video_note_message_id = playing_id;
            return;
        };
        if self.is_applying_snapshot {
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(Duration::from_secs_f64(0.05)).await;
                let _ = this.update_in(cx, |this, window, cx| this.video_note_playback_did_change(window, cx));
            })
            .detach();
            return;
        }
        let touched_ids: HashSet<String> =
            [self.grown_video_note_message_id.take(), playing_id.clone()].into_iter().flatten().collect();
        self.grown_video_note_message_id = playing_id;
        if !rendered_configuration
            .snapshot
            .entries
            .iter()
            .any(|entry| entry.message.as_ref().is_some_and(|message| touched_ids.contains(&message.id)))
        {
            return;
        }

        let anchor = if !self.is_user_scrolling(cx) && self.is_at_exact_bottom(cx) {
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
            true,
            Some(Self::row_slide_animation()),
            window,
            cx,
        );
    }

    pub(in super::super) fn video_note_growth(
        &self,
        entry: &ChatMessageRenderEntry,
        configuration: &RiseMessageCollectionConfiguration,
        width: f32,
        cx: &mut App,
    ) -> f32 {
        let Some(grown_video_note_message_id) = self.grown_video_note_message_id.as_ref() else {
            return 0.0;
        };
        if entry.kind != ChatMessageRenderEntryKind::Message {
            return 0.0;
        }
        let Some(message) = entry.message.as_ref() else {
            return 0.0;
        };
        if &message.id != grown_video_note_message_id
            || MessageContent::bubble_kind(message, Some(configuration.chat.as_ref())) != MessageContentKind::VideoNote
        {
            return 0.0;
        }
        let is_own = MessageList::is_own_bubble(message, &configuration.chat, cx);
        let rest = VideoNoteMessageView::expanded_size(
            width,
            MessageSizes::reserves_avatar_space(&configuration.chat, is_own),
        );
        let playing = VideoNoteMessageView::playing_size(width, &configuration.chat, is_own);
        (playing - rest).max(0.0)
    }

    pub(in super::super) fn update_video_note_floating(&mut self, ids: &[String], cx: &mut Context<Self>) {
        let Some(playing_id) = Self::playing_video_note_message_id(cx) else {
            return;
        };
        let Some(configuration) = self.rendered_configuration.as_ref() else {
            return;
        };
        if !configuration.interactions_enabled || !self.requested_animated_media_playback_active || !self.is_in_window {
            return;
        }
        let entry_id = format!("msg-{playing_id}");
        if !configuration.snapshot.entries_by_id.contains_key(&entry_id) {
            return;
        }
        let store = VideoNotePlayerStore::shared(cx);
        let is_visible = ids.contains(&entry_id);
        if is_visible != store.read(cx).is_floating() {
            return;
        }
        tracing::warn!(target: "riseonly::X1425", is_visible, entry_id = %entry_id, "video note floating flips");
        store.update(cx, |store, cx| {
            if is_visible {
                store.exit_floating(cx);
            } else {
                store.enter_floating(cx);
            }
        });
    }

    pub(in super::super) fn prefetch_directional_media(&mut self, visible_ids: &[String], cx: &mut Context<Self>) {
        let Some(configuration) = self.rendered_configuration.clone().filter(|_| {
            self.requested_animated_media_playback_active
                && self.is_in_window
                && self.window_is_active
                && self.media_prefetch_account_user_id == SaiAccountCacheScope::active_user_id()
                && self.media_prefetch_account_epoch == SaiAccountCacheScope::active_epoch()
        }) else {
            self.cancel_directional_media_prefetch(cx);
            return;
        };
        let snapshot = &configuration.snapshot;
        if self.last_media_prefetch_source_version != Some(snapshot.source_version) {
            self.last_media_prefetch_source_version = Some(snapshot.source_version);
            self.last_media_prefetch_center = None;
        }
        let mut visible_indices: Vec<usize> =
            visible_ids.iter().filter_map(|id| snapshot.indices_by_id.get(id).copied()).collect();
        visible_indices.sort_unstable();
        let (Some(first_visible), Some(last_visible)) = (visible_indices.first().copied(), visible_indices.last().copied())
        else {
            self.cancel_directional_media_prefetch(cx);
            return;
        };
        let center = visible_indices[visible_indices.len() / 2];
        let previous_center = self.last_media_prefetch_center;
        let velocity_y = f32::from(self.list_view.read(cx).scroll_metrics().velocity_y);
        let scroll_speed = velocity_y.abs();
        let moves_toward_newer = if velocity_y != 0.0 {
            velocity_y > 0.0
        } else {
            match previous_center {
                Some(previous_center) if center != previous_center => center > previous_center,
                _ => self.last_media_prefetch_direction_is_newer.unwrap_or(true),
            }
        };
        let center_threshold = if scroll_speed >= 1_100.0 { 6 } else { 3 };
        if let Some(previous_center) = previous_center
            && self.last_media_prefetch_direction_is_newer == Some(moves_toward_newer)
            && previous_center.abs_diff(center) < center_threshold
        {
            return;
        }
        self.last_media_prefetch_center = Some(center);
        self.last_media_prefetch_direction_is_newer = Some(moves_toward_newer);

        let ahead_count: i64 = (16 + (scroll_speed / 1_200.0).min(32.0) as i64).min(48);
        let behind_count: i64 = 12;
        let maximum_candidate_count = (ahead_count + behind_count) as usize;
        let entry_count = snapshot.entries.len() as i64;
        let (ahead_start, ahead_sign, behind_start, behind_sign) = if moves_toward_newer {
            (last_visible as i64, 1, first_visible as i64, -1)
        } else {
            (first_visible as i64, -1, last_visible as i64, 1)
        };
        let candidate_indices: Vec<usize> = (1..=ahead_count)
            .map(|distance| ahead_start + ahead_sign * distance)
            .chain((1..=behind_count).map(|distance| behind_start + behind_sign * distance))
            .filter(|index| (0..entry_count).contains(index))
            .take(maximum_candidate_count)
            .map(|index| index as usize)
            .collect();

        let mut bitmap_urls: Vec<String> = Vec::with_capacity(48);
        let mut lottie_urls: Vec<String> = Vec::with_capacity(48);
        let mut video_urls: Vec<String> = Vec::with_capacity(4);
        let mut bitmap_url_set: HashSet<String> = HashSet::new();
        let mut lottie_url_set: HashSet<String> = HashSet::new();
        let mut video_url_set: HashSet<String> = HashSet::new();

        for index in candidate_indices {
            let Some(message) = snapshot.entries[index].message.as_ref() else {
                continue;
            };
            for item in &message.media_items {
                if item.sticker_kind == Some(MessageStickerKind::Lottie) {
                    rise_message_append_prefetch_url(&mut lottie_urls, &mut lottie_url_set, 48, Some(&item.url));
                    continue;
                }
                if item.kind == MessageMediaKind::Video
                    && item.sticker_kind.is_none()
                    && !self.suspends_animated_media_for_fast_scroll
                {
                    rise_message_append_prefetch_url(&mut video_urls, &mut video_url_set, 4, Some(&item.url));
                }
                if let Some(thumbnail) = item.thumbnail_url.as_deref()
                    && RemoteMediaURL::optional_normalized(Some(thumbnail)).is_some()
                    && !MessageMediaItem::has_video_file_extension(thumbnail)
                {
                    rise_message_append_prefetch_url(&mut bitmap_urls, &mut bitmap_url_set, 48, Some(thumbnail));
                    continue;
                }
                if item.kind == MessageMediaKind::Sticker {
                    rise_message_append_prefetch_url(&mut bitmap_urls, &mut bitmap_url_set, 48, Some(&item.url));
                    continue;
                }
                if let Some(variant) = item
                    .variants
                    .iter()
                    .min_by_key(|variant| variant.size.unwrap_or_else(|| (variant.width * variant.height).max(1)))
                {
                    rise_message_append_prefetch_url(&mut bitmap_urls, &mut bitmap_url_set, 48, Some(&variant.url));
                }
            }
            rise_message_append_prefetch_url(
                &mut bitmap_urls,
                &mut bitmap_url_set,
                48,
                message.link_preview.as_ref().and_then(|preview| preview.image_url.as_deref()),
            );
            if let Some(shared_post) = message.shared_post.as_ref()
                && PostMediaCachePolicy::is_own(None, Some(&shared_post.author.id), cx)
            {
                rise_message_append_prefetch_url(
                    &mut bitmap_urls,
                    &mut bitmap_url_set,
                    48,
                    shared_post.display_thumbnail().as_deref(),
                );
            }
            rise_message_append_prefetch_url(
                &mut bitmap_urls,
                &mut bitmap_url_set,
                48,
                message.contact.as_ref().and_then(|contact| contact.avatar_url.as_deref()),
            );
        }
        self.media_prefetch_generation = self.media_prefetch_generation.wrapping_add(1);
        let generation = self.media_prefetch_generation;
        let scope = self.media_prefetch_scope.clone();
        let account_user_id = self.media_prefetch_account_user_id.clone();
        let account_epoch = self.media_prefetch_account_epoch.clone();

        CleverImageDiskPrefetch::replace_directional(
            bitmap_urls.into_iter().map(CleverImagePrefetchItem::new).collect(),
            &scope,
            generation,
            account_user_id.clone(),
            account_epoch.clone(),
            cx,
        );
        VideoWarmStartPrefetch::replace_directional(
            &video_urls,
            &scope,
            generation,
            account_user_id.clone(),
            account_epoch.clone(),
            cx,
        );
        LottieStickerPrefetch::replace_directional(&lottie_urls, &scope, generation, account_user_id, account_epoch, cx);
    }

    pub(in super::super) fn cancel_directional_media_prefetch(&mut self, cx: &mut Context<Self>) {
        if self.last_media_prefetch_center.is_none() && self.last_media_prefetch_source_version.is_none() {
            return;
        }
        self.media_prefetch_generation = self.media_prefetch_generation.wrapping_add(1);
        let generation = self.media_prefetch_generation;
        CleverImageDiskPrefetch::cancel_directional(&self.media_prefetch_scope, generation, cx);
        LottieStickerPrefetch::cancel_directional(&self.media_prefetch_scope, generation, cx);
        VideoWarmStartPrefetch::cancel_directional(&self.media_prefetch_scope, generation, cx);
        self.last_media_prefetch_center = None;
        self.last_media_prefetch_direction_is_newer = None;
        self.last_media_prefetch_source_version = None;
    }

    pub(in super::super) fn make_native_predisplay_requests(
        inputs: Vec<RiseMessageManualListViewNativePredisplayInput>,
    ) -> Vec<AsyncTextBubblePredisplayRequest> {
        inputs
            .into_iter()
            .map(|input| {
                AsyncTextBubblePredisplayRequest::new(input.prepared_layout, input.style, input.corners, input.scale)
            })
            .collect()
    }

    pub(in super::super) fn predisplay_inputs(
        &self,
        candidate: &RiseMessageManualListViewPreparedSnapshotCandidate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<RiseMessageManualListViewNativePredisplayInput> {
        let entry_count = candidate.configuration.snapshot.entries.len();
        let request_limit = if self.initial_frame_ready_delivered {
            24
        } else {
            RiseFallbackMeasurementPolicy::NEARBY_ENTRY_LIMIT as usize
        };
        let centered_range = |center: usize| -> Range<usize> {
            let lower_bound = center.saturating_sub(request_limit / 2).min(entry_count.saturating_sub(request_limit));
            lower_bound..entry_count.min(lower_bound + request_limit)
        };
        let indices_by_id = &candidate.configuration.snapshot.indices_by_id;

        if !self.initial_frame_ready_delivered
            && self.did_restore_initial_position
            && !self.pending_scroll_reset
            && self.pending_scroll_request.is_none()
        {
            let list_view = self.list_view.read(cx);
            let first_visible = list_view.visible_ids().first().and_then(|id| indices_by_id.get(id).copied());
            let last_visible = list_view.visible_ids().last().and_then(|id| indices_by_id.get(id).copied());
            if let (Some(first_visible), Some(last_visible)) = (first_visible, last_visible) {
                let visible_center = first_visible + last_visible.saturating_sub(first_visible) / 2;
                return self.predisplay_inputs_in_range(candidate, centered_range(visible_center), cx);
            }
        }
        if (!self.did_restore_initial_position || self.pending_scroll_reset)
            && let Some(center) = self.initial_predisplay_center_index(candidate, window, cx)
        {
            return self.predisplay_inputs_in_range(candidate, centered_range(center), cx);
        }
        let anchor_id = self.list_view.update(cx, |list_view, _| list_view.capture_visible_anchor()).map(|anchor| anchor.id);
        if let Some(anchor_index) = anchor_id.and_then(|id| indices_by_id.get(&id).copied()) {
            return self.predisplay_inputs_in_range(candidate, centered_range(anchor_index), cx);
        }
        let entry_range = if candidate.configuration.initial_scroll_position == RiseMessageInitialScrollPosition::Bottom {
            entry_count.saturating_sub(request_limit)..entry_count
        } else {
            0..entry_count.min(request_limit)
        };
        self.predisplay_inputs_in_range(candidate, entry_range, cx)
    }

    pub(in super::super) fn deferred_predisplay_inputs(
        &self,
        candidate: &RiseMessageManualListViewPreparedSnapshotCandidate,
        cx: &App,
    ) -> Vec<RiseMessageManualListViewNativePredisplayInput> {
        let entry_count = candidate.configuration.snapshot.entries.len();
        let chunk = &candidate.native_batch.entry_range;
        let lower_bound = entry_count.min(chunk.start);
        let upper_bound = lower_bound.max(entry_count.min(chunk.end));
        if lower_bound >= upper_bound {
            return Vec::new();
        }

        let request_limit = (upper_bound - lower_bound).min(24);
        let selected_range = if let Some(visible_range) = self.deferred_history_visible_range.as_ref() {
            if upper_bound <= visible_range.start {
                lower_bound.max(upper_bound - request_limit)..upper_bound
            } else if lower_bound >= visible_range.end {
                lower_bound..upper_bound.min(lower_bound + request_limit)
            } else {
                let center = (upper_bound - 1).min(lower_bound.max(visible_range.start + visible_range.len() / 2));
                let selected_lower_bound =
                    lower_bound.max(center.saturating_sub(request_limit / 2)).min(upper_bound - request_limit);
                selected_lower_bound..selected_lower_bound + request_limit
            }
        } else if upper_bound <= self.rendered_foreground_preparation_range.start {
            lower_bound.max(upper_bound - request_limit)..upper_bound
        } else {
            lower_bound..upper_bound.min(lower_bound + request_limit)
        };
        self.predisplay_inputs_in_range(candidate, selected_range, cx)
    }

    pub(in super::super) fn lottie_predisplay_urls(
        &self,
        candidate: &RiseMessageManualListViewPreparedSnapshotCandidate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<String> {
        let entries = &candidate.configuration.snapshot.entries;
        if entries.is_empty() {
            return Vec::new();
        }
        let center = self.initial_predisplay_center_index(candidate, window, cx).unwrap_or(entries.len() - 1);
        let lower_bound = center.saturating_sub(8);
        let upper_bound = entries.len().min(center + 17);
        let mut seen: HashSet<String> = HashSet::new();
        let mut result: Vec<String> = Vec::with_capacity(12);
        for entry in &entries[lower_bound.min(upper_bound)..upper_bound] {
            let Some(url) = entry.message.as_ref().and_then(|message| message.lottie_sticker_prefetch_url()) else {
                continue;
            };
            if !seen.insert(url.clone()) {
                continue;
            }
            result.push(url);
            if result.len() == 12 {
                break;
            }
        }
        result
    }

    pub(in super::super) fn bitmap_predisplay_urls(
        &self,
        candidate: &RiseMessageManualListViewPreparedSnapshotCandidate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<String> {
        let entries = &candidate.configuration.snapshot.entries;
        if entries.is_empty() {
            return Vec::new();
        }
        let center = self.initial_predisplay_center_index(candidate, window, cx).unwrap_or(entries.len() - 1);
        let lower_bound = center.saturating_sub(8);
        let upper_bound = entries.len().min(center + 17);
        let mut seen: HashSet<String> = HashSet::new();
        let mut result: Vec<String> = Vec::with_capacity(12);
        let mut append = |raw: Option<&str>| {
            if result.len() >= 12 {
                return;
            }
            let Some(value) = RemoteMediaURL::optional_normalized(raw) else {
                return;
            };
            if MessageMediaItem::has_video_file_extension(&value) || !seen.insert(value.clone()) {
                return;
            }
            result.push(value);
        };
        for entry in &entries[lower_bound.min(upper_bound)..upper_bound] {
            let Some(message) = entry.message.as_ref() else {
                continue;
            };
            for item in &message.media_items {
                append(item.thumbnail_url.as_deref());
                if item.sticker_kind == Some(MessageStickerKind::Lottie) {
                    continue;
                }
                if item.kind == MessageMediaKind::Sticker {
                    append(Some(&item.url));
                } else if let Some(variant) = item
                    .variants
                    .iter()
                    .min_by_key(|variant| variant.size.unwrap_or_else(|| (variant.width * variant.height).max(1)))
                {
                    append(Some(&variant.url));
                }
            }
            append(message.link_preview.as_ref().and_then(|preview| preview.image_url.as_deref()));
            append(message.contact.as_ref().and_then(|contact| contact.avatar_url.as_deref()));
        }
        result
    }

    pub(in super::super) fn predisplay_inputs_in_range(
        &self,
        candidate: &RiseMessageManualListViewPreparedSnapshotCandidate,
        entry_range: Range<usize>,
        cx: &App,
    ) -> Vec<RiseMessageManualListViewNativePredisplayInput> {
        let configuration = &candidate.configuration;
        let entries = &configuration.snapshot.entries;
        let scale = self.display_scale;
        let cache = AsyncTextBubblePredisplayCache::shared();
        let theme = configuration.theme_store.read(cx).current_theme().clone();
        let mut inputs: Vec<RiseMessageManualListViewNativePredisplayInput> = Vec::with_capacity(entry_range.len());
        for entry in &entries[entry_range] {
            if candidate.native_batch.versions.get(&entry.id) != Some(&entry.content_version) {
                continue;
            }
            let (Some(layout), Some(is_own), Some(group)) = (
                candidate.native_batch.layouts.get(&entry.id),
                candidate.native_batch.ownership_by_id.get(&entry.id).copied(),
                entry.group,
            ) else {
                continue;
            };
            if !cache.can_predisplay(layout, scale) {
                continue;
            }
            let (style, corners) = RiseNativeTextMessageCell::appearance(&theme, is_own, group.is_first, group.is_last);
            inputs.push(RiseMessageManualListViewNativePredisplayInput {
                prepared_layout: layout.clone(),
                style,
                corners,
                scale,
            });
        }
        inputs
    }

    pub(in super::super) fn initial_predisplay_center_index(
        &self,
        candidate: &RiseMessageManualListViewPreparedSnapshotCandidate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<usize> {
        let entries = &candidate.configuration.snapshot.entries;
        if entries.is_empty() {
            return None;
        }
        if let Some(request) = self.pending_scroll_request.as_ref() {
            match &request.destination {
                RiseMessageScrollDestination::Takeover => {}
                RiseMessageScrollDestination::Latest => return Some(entries.len() - 1),
                RiseMessageScrollDestination::Message(message_id) => {
                    if let Some(index) = entries.iter().position(|entry| {
                        entry.message.as_ref().is_some_and(|message| message.matches_message_id(message_id))
                    }) {
                        return Some(index);
                    }
                }
                RiseMessageScrollDestination::DayStart(day_start_ms) => {
                    if let Some(index) = entries.iter().position(|entry| {
                        entry.kind == ChatMessageRenderEntryKind::DateSeparator
                            && entry.day_start_ms == Some(*day_start_ms)
                    }) {
                        return Some(index);
                    }
                }
            }
        }
        if candidate.configuration.initial_scroll_position != RiseMessageInitialScrollPosition::Bottom {
            return Some(0);
        }
        let Some(position) = candidate.configuration.saved_scroll_position(cx) else {
            return Some(
                entries
                    .iter()
                    .rposition(|entry| entry.kind == ChatMessageRenderEntryKind::UnreadSeparator)
                    .unwrap_or(entries.len() - 1),
            );
        };

        let top_inset = (candidate.configuration.top_inset + 10.0).max(10.0);
        if let Some(anchor_index) = position.anchor_id.as_deref().and_then(|anchor_id| {
            RiseMessageScrollAnchorResolution::candidates(anchor_id)
                .iter()
                .find_map(|id| candidate.configuration.snapshot.indices_by_id.get(id).copied())
        }) {
            return Some(anchor_index);
        }
        let target_y = (position.distance_from_top - top_inset + f32::from(self.bounds.size.height) * 0.5).max(0.0);
        let mut next_y: f32 = 0.0;
        for (index, entry) in entries.iter().enumerate() {
            if index > 0 {
                next_y += MessageSizes::list_row_spacing();
            }
            let height = self.prepared_row_height(entry, candidate, window, cx);
            if next_y + height > target_y || index == entries.len() - 1 {
                return Some(index);
            }
            next_y += height;
        }
        Some(entries.len() - 1)
    }
}
