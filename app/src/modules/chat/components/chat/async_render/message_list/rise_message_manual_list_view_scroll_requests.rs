use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;
use gpui::{App, Context, Hsla, Window};
use rise_core::foundation::foundation_date::Date;
use rise_theme::theme::app_theme::color;
use rise_ui::swiftui::environment_values::ColorScheme;
use crate::modules::chat::engine::messages::rise_chat_calendar::RiseChatCalendarWindow;
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_types::RiseManualListTransactionJumpAnimationDirection;
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_types::RiseManualListViewportAnchor;
use crate::modules::chat::components::chat::async_render::chat_message_render_snapshot::{ChatMessageRenderEntryKind, ChatMessageRenderSnapshot};
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_types::RiseManualListTransactionAnchor;
use crate::modules::chat::components::chat::scroll::chat_scroll_position_store::{ChatScrollPositionStore, ChatScrollPositionStorePosition};
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::chat::stores::message::message_actions::message_types::GetMessageMessage;
use super::super::rise_message_list_view::*;

impl RiseMessageManualListView {
    pub(in super::super) fn unread_separator_anchor(
        &self,
        configuration: &RiseMessageCollectionConfiguration,
        item_table: &RiseMessageManualListViewPreparedManualItemTable,
    ) -> Option<RiseManualListTransactionAnchor> {
        let ids = &configuration.snapshot.ids;
        let index = ids.iter().position(|id| {
            configuration
                .snapshot
                .entries_by_id
                .get(id)
                .is_some_and(|entry| entry.kind == ChatMessageRenderEntryKind::UnreadSeparator)
        })?;
        let top_inset = (configuration.top_inset + 10.0).max(10.0);
        let bottom_inset = (configuration.bottom_inset + 10.0).max(10.0);
        let visible_height = f32::from(self.bounds.size.height) - top_inset - bottom_inset;
        let spacing = MessageSizes::list_row_spacing();
        let mut tail_height = 0.0;
        for id in &ids[index..] {
            tail_height += item_table.items_by_id.get(id).map_or(64.0, |item| item.height) + spacing;
            if tail_height > visible_height {
                return Some(RiseManualListTransactionAnchor::Item {
                    id: ids[index].clone(),
                    viewport_offset: top_inset,
                });
            }
        }
        None
    }

    pub(in super::super) fn scroll_request_target_is_prepared(
        &self,
        configuration: &RiseMessageCollectionConfiguration,
        item_table: &RiseMessageManualListViewPreparedManualItemTable,
    ) -> bool {
        let Some(request) = self.pending_scroll_request.as_ref() else {
            return true;
        };
        let entry_id = match &request.destination {
            RiseMessageScrollDestination::Takeover | RiseMessageScrollDestination::Latest => return true,
            RiseMessageScrollDestination::Message(message_id) => self.message_entry_id(message_id, configuration),
            RiseMessageScrollDestination::DayStart(day_start_ms) => {
                self.date_separator_entry_id(*day_start_ms, configuration)
            }
        };
        entry_id.is_none_or(|entry_id| item_table.items_by_id.contains_key(&entry_id))
    }

    pub(in super::super) fn preferred_visible_anchor(&self, cx: &mut App) -> Option<RiseManualListViewportAnchor> {
        let list_view = self.list_view.read(cx);
        for id in list_view.visible_ids() {
            let Some(kind) = self.entries_by_id.get(id).map(|entry| &entry.kind) else {
                continue;
            };
            if (*kind == ChatMessageRenderEntryKind::Message || *kind == ChatMessageRenderEntryKind::System)
                && let Some(anchor) = list_view.capture_viewport_anchor(id)
            {
                return Some(anchor);
            }
        }
        self.list_view.update(cx, |list_view, _| list_view.capture_visible_anchor())
    }

    pub(in super::super) fn removal_anchor(
        &self,
        removed_ids: &HashSet<String>,
        previous_snapshot: Option<&ChatMessageRenderSnapshot>,
        new_id_set: &HashSet<String>,
        can_pin_bottom: bool,
        cx: &App,
    ) -> Option<RiseManualListTransactionAnchor> {
        if removed_ids.is_empty() {
            return None;
        }
        let previous_snapshot = previous_snapshot?;
        let list_view = self.list_view.read(cx);
        let mut surviving_anchor: Option<RiseManualListViewportAnchor> = None;
        for id in list_view.visible_ids().iter().rev().filter(|id| new_id_set.contains(*id)) {
            let Some(kind) = previous_snapshot.entries_by_id.get(id).map(|entry| &entry.kind) else {
                continue;
            };
            if *kind != ChatMessageRenderEntryKind::Message && *kind != ChatMessageRenderEntryKind::System {
                continue;
            }
            let Some(captured) = list_view.capture_viewport_anchor(id) else {
                continue;
            };
            surviving_anchor = Some(captured);
            break;
        }
        let bottom_fallback = || can_pin_bottom.then_some(RiseManualListTransactionAnchor::Bottom);
        let Some(surviving_anchor) = surviving_anchor else {
            return bottom_fallback();
        };
        let Some(anchor_index) = previous_snapshot.indices_by_id.get(&surviving_anchor.id).copied() else {
            return bottom_fallback();
        };
        let item_anchor = RiseManualListTransactionAnchor::Item {
            id: surviving_anchor.id,
            viewport_offset: surviving_anchor.viewport_offset,
        };
        if !can_pin_bottom {
            return Some(item_anchor);
        }
        let inter_item_spacing = f32::from(list_view.inter_item_spacing());
        let mut removed_height_below_anchor: f32 = 0.0;
        for id in removed_ids {
            let Some(index) = previous_snapshot.indices_by_id.get(id).copied() else {
                continue;
            };
            if index <= anchor_index {
                continue;
            }
            let Some(item) = self.manual_items_by_id.get(id) else {
                continue;
            };
            removed_height_below_anchor += item.height + inter_item_spacing;
        }
        let distance_from_bottom = f32::from(list_view.scroll_metrics().distance_from_bottom);
        let opens_gap_at_bottom = removed_height_below_anchor > 0.0
            && removed_height_below_anchor + RiseMessageScrollPositionPersistencePolicy::EXACT_BOTTOM_TOLERANCE
                >= distance_from_bottom;
        if opens_gap_at_bottom {
            return Some(RiseManualListTransactionAnchor::Bottom);
        }
        Some(item_anchor)
    }

    pub(in super::super) fn persisted_visible_anchor(&self, cx: &mut App) -> Option<RiseManualListViewportAnchor> {
        let list_view = self.list_view.read(cx);
        for id in list_view.visible_ids() {
            if (id.starts_with("msg-") || id.starts_with("sys-") || id.starts_with("topic-"))
                && let Some(anchor) = list_view.capture_viewport_anchor(id)
            {
                return Some(anchor);
            }
        }
        self.list_view.update(cx, |list_view, _| list_view.capture_visible_anchor())
    }

    pub fn persist_scroll_position(
        &mut self,
        configuration: Option<&RiseMessageCollectionConfiguration>,
        cx: &mut Context<Self>,
    ) {
        if let Some(configuration) = configuration {
            self.persist_scroll_position_for(configuration, cx);
            return;
        }
        if !self.did_restore_initial_position {
            tracing::warn!(target: "riseonly::X696", "persist skipped not restored key={:?} frame_ready={}", self.rendered_configuration.as_ref().or(self.configuration.as_ref()).map(|configuration| configuration.scroll_storage_key()), self.initial_frame_ready_delivered);
            return;
        }
        let Some(configuration) = self.rendered_configuration.clone().or_else(|| self.configuration.clone()) else {
            return;
        };
        self.persist_scroll_position_for(&configuration, cx);
    }

    pub(in super::super) fn persist_scroll_position_for(
        &mut self,
        configuration: &RiseMessageCollectionConfiguration,
        cx: &mut Context<Self>,
    ) {
        let scroll_storage_key = configuration.scroll_storage_key();
        tracing::warn!(target: "riseonly::X696", "persist key={scroll_storage_key} frame_ready={} restored={} alpha={} persists={}", self.initial_frame_ready_delivered, self.did_restore_initial_position, self.list_alpha, configuration.persists_scroll_position);
        if !self.initial_frame_ready_delivered
            || !self.did_restore_initial_position
            || self.list_alpha <= 0.01
            || !configuration.persists_scroll_position
            || scroll_storage_key.is_empty()
        {
            return;
        }
        let metrics = self.list_view.read(cx).scroll_metrics();
        let anchor = self.persisted_visible_anchor(cx);
        let viewport_offset = anchor.as_ref().map_or(0.0, |anchor| anchor.viewport_offset);
        ChatScrollPositionStore::save_position(
            ChatScrollPositionStorePosition::new(
                anchor.map(|anchor| anchor.id),
                viewport_offset,
                f32::from(metrics.distance_from_top),
            ),
            &scroll_storage_key,
            RiseMessageScrollPositionPersistencePolicy::discards_saved_position(
                f32::from(metrics.distance_from_bottom),
                configuration.can_load_newer,
            ),
            cx,
        );
        if let Some(fallback_key) = configuration.scroll_storage_fallback_key() {
            ChatScrollPositionStore::clear(&fallback_key, cx);
        }
    }

    /// dequeues the pending scroll request and then performs it, in that order
    /// a smart scroll reaches the target synchronously and metrics return here through the flush
    /// if not performed it goes back
    pub(in super::super) fn consume_pending_scroll_request(
        &mut self,
        configuration: &Rc<RiseMessageCollectionConfiguration>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(request) = self.pending_scroll_request.take() else {
            return;
        };
        let was_reset_pending = self.pending_scroll_reset;
        self.pending_scroll_reset = false;

        if !self.perform_scroll_request(&request, configuration, window, cx) {
            self.pending_scroll_request = Some(request);
            self.pending_scroll_reset = was_reset_pending;
            return;
        }
        self.did_restore_initial_position = true;
    }

    pub(in super::super) fn perform_scroll_request(
        &mut self,
        request: &RiseMessageScrollRequest,
        configuration: &RiseMessageCollectionConfiguration,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let source_version = configuration.snapshot.source_version;
        if !self.rendered_configuration.as_ref().is_some_and(|rendered| {
            rendered.chat_id == configuration.chat_id && rendered.snapshot.source_version == source_version
        }) || self.applied_source_version != Some(source_version)
        {
            return false;
        }

        let animated = request.animated;
        match &request.destination {
            RiseMessageScrollDestination::Takeover => true,
            RiseMessageScrollDestination::Latest => {
                if configuration.can_load_newer {
                    return false;
                }
                self.list_view.update(cx, |list_view, cx| list_view.smart_scroll_to_bottom(animated, window, cx))
            }
            RiseMessageScrollDestination::Message(message_id) => {
                let Some(entry_id) = self.message_entry_id(message_id, configuration) else {
                    return false;
                };
                let direction = self.jump_direction(request, configuration, cx);
                let performed = self.list_view.update(cx, |list_view, cx| {
                    list_view.smart_scroll_to_item_centered(&entry_id, direction, animated, window, cx)
                });
                if performed {
                    self.flash_jump_highlight(entry_id, Self::jump_highlight_color(configuration), window, cx);
                }
                performed
            }
            RiseMessageScrollDestination::DayStart(day_start_ms) => {
                let Some(entry_id) = self.date_separator_entry_id(*day_start_ms, configuration) else {
                    return false;
                };
                let direction = self.jump_direction(request, configuration, cx);
                self.list_view.update(cx, |list_view, cx| {
                    list_view.smart_scroll_to_item_at_top(&entry_id, direction, animated, window, cx)
                })
            }
        }
    }

    pub(in super::super) fn date_separator_entry_id(
        &self,
        day_start_ms: i64,
        configuration: &RiseMessageCollectionConfiguration,
    ) -> Option<String> {
        configuration
            .snapshot
            .entries
            .iter()
            .find(|entry| {
                entry.kind == ChatMessageRenderEntryKind::DateSeparator && entry.day_start_ms == Some(day_start_ms)
            })
            .map(|entry| entry.id.clone())
    }

    pub(in super::super) fn trigger_jump_highlight(
        &mut self,
        request: &RiseMessageScrollRequest,
        configuration: &Rc<RiseMessageCollectionConfiguration>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let RiseMessageScrollDestination::Message(message_id) = &request.destination else {
            return;
        };
        let Some(entry_id) = self.message_entry_id(message_id, configuration) else {
            return;
        };
        self.flash_jump_highlight(entry_id, Self::jump_highlight_color(configuration), window, cx);
    }

    /// washes the landed row now or at the reveal when the list is still hidden
    /// the wash holds under two seconds, about how long a first frame can take
    pub(in super::super) fn flash_jump_highlight(&mut self, entry_id: String, color: Hsla, window: &mut Window, cx: &mut Context<Self>) {
        if !self.initial_frame_ready_delivered {
            self.pending_reveal_highlight = Some((entry_id, color));
            return;
        }
        self.pending_reveal_highlight = None;
        self.list_view.update(cx, |list_view, cx| {
            list_view.flash_row_highlight(&entry_id, color, 0.9, 0.6, window, cx);
        });
    }

    /// light gray that survives both themes and custom palettes, text color at low alpha
    /// the chat wash sits on a busy wallpaper so it needs about twice the alpha
    pub(in super::super) fn jump_highlight_color(configuration: &RiseMessageCollectionConfiguration) -> Hsla {
        Hsla {
            a: if configuration.color_scheme == ColorScheme::Dark { 0.18 } else { 0.20 },
            ..color(&configuration.theme_palette.text_100)
        }
    }

    pub(in super::super) fn message_entry_id(
        &self,
        message_id: &str,
        configuration: &RiseMessageCollectionConfiguration,
    ) -> Option<String> {
        configuration
            .snapshot
            .entries
            .iter()
            .find(|entry| entry.message.as_ref().is_some_and(|message| message.matches_message_id(message_id)))
            .map(|entry| entry.id.clone())
    }

    pub(in super::super) fn jump_direction(
        &self,
        request: &RiseMessageScrollRequest,
        configuration: &RiseMessageCollectionConfiguration,
        cx: &App,
    ) -> RiseManualListTransactionJumpAnimationDirection {
        let target = match &request.destination {
            RiseMessageScrollDestination::Message(message_id) => {
                configuration.messages.iter().find(|message| message.matches_message_id(message_id))
            }
            RiseMessageScrollDestination::DayStart(day_start_ms) => configuration.messages.iter().find(|message| {
                RiseChatCalendarWindow::milliseconds(&Date::from_time_interval_since_1970(message.created_at))
                    >= *day_start_ms
            }),
            RiseMessageScrollDestination::Takeover | RiseMessageScrollDestination::Latest => None,
        };
        let Some(target) = target else {
            return RiseManualListTransactionJumpAnimationDirection::TowardNewer;
        };

        let current_configuration: &RiseMessageCollectionConfiguration =
            self.rendered_configuration.as_deref().unwrap_or(configuration);
        let visible_messages: Vec<&Arc<GetMessageMessage>> = self
            .list_view
            .read(cx)
            .visible_ids()
            .iter()
            .filter_map(|id| {
                let index = current_configuration.snapshot.indices_by_id.get(id).copied()?;
                current_configuration.snapshot.entries[index].message.as_ref()
            })
            .collect();
        let reference = if visible_messages.is_empty() {
            current_configuration.messages.get(current_configuration.messages.len() / 2)
        } else {
            Some(visible_messages[visible_messages.len() / 2])
        };
        let Some(reference) = reference else {
            return RiseManualListTransactionJumpAnimationDirection::TowardOlder;
        };
        if target.created_at < reference.created_at {
            RiseManualListTransactionJumpAnimationDirection::TowardOlder
        } else {
            RiseManualListTransactionJumpAnimationDirection::TowardNewer
        }
    }

    pub(in super::super) fn scroll_to_bottom(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_user_scrolling(cx)
            || self
                .rendered_configuration
                .as_ref()
                .or(self.configuration.as_ref())
                .is_some_and(|configuration| configuration.can_load_newer)
        {
            return;
        }
        self.list_view.update(cx, |list_view, cx| {
            list_view.scroll_to_bottom(false, window, cx);
        });
    }
}
