use gpui::{App, Context, Edges, Pixels, Window, px};
use crate::modules::chat::components::chat::async_render::chat_message_render_snapshot::ChatMessageRenderEntryKind;
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_types::RiseManualListTransactionAnchor;
use crate::modules::chat::components::chat::scroll::chat_messages_scroll_controller::ChatComposerInsetAnimation;
use crate::modules::chat::components::chat::composer::chat_composer_types::ChatBottomBarMetrics;
use super::super::rise_message_list_view::*;

impl RiseMessageManualListView {
    pub(in super::super) fn capture_viewport_resize_anchor(
        &self,
        width_delta: f32,
        height_delta: f32,
        cx: &App,
    ) -> Option<RiseManualListTransactionAnchor> {
        if self.is_at_exact_bottom(cx) {
            return Some(RiseManualListTransactionAnchor::Bottom);
        }
        let list_view = self.list_view.read(cx);
        if width_delta > 0.5 && height_delta > 0.5 {
            for id in list_view.visible_ids().iter() {
                let Some(kind) = self.entries_by_id.get(id).map(|entry| entry.kind) else {
                    continue;
                };
                if kind != ChatMessageRenderEntryKind::Message && kind != ChatMessageRenderEntryKind::System {
                    continue;
                }
                let Some(anchor) = list_view.capture_viewport_anchor(id) else {
                    continue;
                };
                return Some(RiseManualListTransactionAnchor::Item {
                    id: anchor.id,
                    viewport_offset: anchor.viewport_offset,
                });
            }
        }
        for id in list_view.visible_ids().iter().rev() {
            let Some(kind) = self.entries_by_id.get(id).map(|entry| entry.kind) else {
                continue;
            };
            if kind != ChatMessageRenderEntryKind::Message && kind != ChatMessageRenderEntryKind::System {
                continue;
            }
            let Some(anchor) = list_view.capture_viewport_anchor(id) else {
                continue;
            };
            return Some(RiseManualListTransactionAnchor::Item {
                id: anchor.id,
                viewport_offset: anchor.viewport_offset + height_delta,
            });
        }
        None
    }

    pub(in super::super) fn current_keyboard_guide_overlap(&self) -> f32 {
        0.0
    }

    pub(in super::super) fn resolved_configured_viewport_insets(&self, keyboard_overlap: f32) -> (Edges<Pixels>, Option<Pixels>) {
        let active_overlap = if self.tracks_system_keyboard { keyboard_overlap.max(0.0) } else { 0.0 };
        let resting_bottom_inset = RiseMessageKeyboardRestingInsetPolicy::resolved_inset(
            self.configured_viewport_bottom_inset,
            self.configured_keyboard_resting_bottom_inset,
            active_overlap,
            self.keyboard_target_is_visible,
        );
        let bottom_inset = RiseMessageKeyboardViewportPolicy::resolved_inset(
            self.configured_viewport_bottom_inset,
            resting_bottom_inset,
            active_overlap,
            self.tracks_system_keyboard,
            self.configured_keyboard_active_bottom_inset,
        );
        let sticky_inset = match (
            self.tracks_system_keyboard,
            self.configured_sticky_bottom_inset,
            self.configured_keyboard_resting_sticky_bottom_inset,
        ) {
            (true, Some(configured_sticky), Some(resting_sticky)) => {
                let resting_sticky_bottom_inset = RiseMessageKeyboardRestingInsetPolicy::resolved_inset(
                    configured_sticky,
                    resting_sticky,
                    active_overlap,
                    self.keyboard_target_is_visible,
                );
                Some(RiseMessageKeyboardViewportPolicy::resolved_inset(
                    configured_sticky,
                    resting_sticky_bottom_inset,
                    active_overlap,
                    true,
                    self.configured_keyboard_active_sticky_bottom_inset,
                ))
            }
            _ => self.configured_sticky_bottom_inset,
        };
        (
            Edges {
                top: px(self.configured_viewport_top_inset.max(0.0).round()),
                right: px(0.0),
                bottom: px(bottom_inset.max(0.0).round()),
                left: px(0.0),
            },
            sticky_inset.map(|inset| px(inset.max(0.0).round())),
        )
    }

    pub(in super::super) fn animate_viewport_transition_to_composer_timing(
        &mut self,
        target: (Edges<Pixels>, Option<Pixels>),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.list_view.update(cx, |list_view, cx| {
            list_view.animate_keyboard_viewport_transition(
                target.0,
                target.1,
                ChatComposerInsetAnimation::last_duration(),
                window,
                cx,
            );
        });
    }

    pub(in super::super) fn finish_viewport_transition(
        &mut self,
        target: (Edges<Pixels>, Option<Pixels>),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.list_view.update(cx, |list_view, cx| {
            list_view.finish_keyboard_viewport_transition(target.0, target.1, window, cx);
        });
    }

    pub(in super::super) fn animate_configured_composer_transition(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.composer_release_awaits_sent_row = false;
        let target = self.resolved_configured_viewport_insets(0.0);
        self.animate_viewport_transition_to_composer_timing(target, window, cx);
    }

    pub(in super::super) fn finish_configured_keyboard_transition(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.keeps_composer_release_held_for_unchanged_keyboard(cx) {
            return;
        }
        self.composer_release_awaits_sent_row = false;
        let target = self.resolved_configured_viewport_insets(self.keyboard_target_overlap);
        self.finish_viewport_transition(target, window, cx);
    }

    pub(in super::super) fn hold_composer_release_for_sent_row(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !ChatComposerInsetAnimation::announces_send()
            || self.list_view.read(cx).has_active_keyboard_viewport_transition()
            || !self.is_at_exact_bottom(cx)
        {
            return false;
        }
        self.composer_release_awaits_sent_row = true;
        self.composer_release_held_keyboard_overlap = self.keyboard_target_overlap;
        self.composer_release_generation = self.composer_release_generation.wrapping_add(1);
        let generation = self.composer_release_generation;
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(ChatComposerInsetAnimation::SENT_ROW_HOLD_DURATION)
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                if !this.composer_release_awaits_sent_row || this.composer_release_generation != generation {
                    return;
                }
                ChatComposerInsetAnimation::set_last_duration(ChatBottomBarMetrics::COMPOSER_ANIM_DURATION);
                this.release_held_composer_inset(true, window, cx);
            });
        })
        .detach();
        true
    }

    pub(in super::super) fn keeps_composer_release_held_for_unchanged_keyboard(&self, cx: &App) -> bool {
        if !self.composer_release_awaits_sent_row || !self.is_in_window {
            return false;
        }
        if (self.keyboard_target_overlap - self.composer_release_held_keyboard_overlap).abs() <= 0.5 {
            return true;
        }
        if !self.keyboard_target_is_visible {
            return false;
        }
        self.held_composer_release_would_lower_rows(cx)
    }

    /// while the sent row is on its way, a release that only lowers the bottom inset would move
    /// the pinned column down a few frames
    /// before the row lifts it back
    pub(in super::super) fn held_composer_release_would_lower_rows(&self, cx: &App) -> bool {
        let target = self.resolved_configured_viewport_insets(self.keyboard_target_overlap);
        target.0.bottom <= self.list_view.read(cx).viewport_insets().bottom + px(0.5)
    }

    /// ends the hold and returns the insets the release lands on, none when nothing moves
    pub(in super::super) fn take_held_composer_release_target(&mut self, cx: &App) -> Option<(Edges<Pixels>, Option<Pixels>)> {
        if !self.composer_release_awaits_sent_row {
            return None;
        }
        self.composer_release_awaits_sent_row = false;
        let list_view = self.list_view.read(cx);
        if list_view.has_active_keyboard_viewport_transition() {
            return None;
        }
        let target = self.resolved_configured_viewport_insets(if self.tracks_system_keyboard {
            self.keyboard_target_overlap
        } else {
            0.0
        });
        if list_view.viewport_insets() == target.0 && list_view.bottom_sticky_viewport_inset() == target.1 {
            return None;
        }
        Some(target)
    }

    pub(in super::super) fn release_held_composer_inset(&mut self, animated: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.take_held_composer_release_target(cx) else {
            return;
        };
        if animated {
            self.animate_viewport_transition_to_composer_timing(target, window, cx);
        } else {
            self.finish_viewport_transition(target, window, cx);
        }
    }

    pub(in super::super) fn apply_configured_viewport_insets(
        &mut self,
        configuration: Option<&RiseMessageCollectionConfiguration>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut composer_bottom_inset_changed = false;
        let settles_immediately = !self.initial_frame_ready_delivered
            || configuration.is_some_and(|configuration| configuration.settles_composer_inset_immediately);
        if let Some(configuration) = configuration {
            self.list_view.update(cx, |list_view, cx| {
                list_view.set_prevents_bottom_overscroll(!configuration.can_load_newer, cx);
                list_view.set_top_sticky_viewport_inset(Some(px(configuration.top_inset.max(0.0))), window, cx);
            });
            let previous_configured_bottom_inset = self.configured_viewport_bottom_inset;
            self.configured_viewport_top_inset = (configuration.top_inset + 10.0).max(10.0);
            self.configured_viewport_bottom_inset = (configuration.bottom_inset + 10.0).max(10.0);
            composer_bottom_inset_changed =
                (previous_configured_bottom_inset - self.configured_viewport_bottom_inset).abs() > 0.5;
            self.configured_keyboard_resting_bottom_inset = self.configured_viewport_bottom_inset.max(
                configuration
                    .keyboard_resting_bottom_inset
                    .unwrap_or(configuration.bottom_inset)
                    + 10.0,
            );
            self.configured_keyboard_active_bottom_inset = configuration
                .keyboard_active_bottom_inset
                .map(|inset| (inset + 10.0).max(10.0));
            self.configured_sticky_bottom_inset = configuration.sticky_bottom_inset.map(|inset| inset.max(0.0));
            self.configured_keyboard_active_sticky_bottom_inset = configuration
                .keyboard_active_sticky_bottom_inset
                .map(|inset| inset.max(0.0));
            self.configured_keyboard_resting_sticky_bottom_inset = configuration
                .keyboard_resting_sticky_bottom_inset
                .or(configuration.sticky_bottom_inset)
                .map(|inset| inset.max(0.0));
            let was_tracking = self.tracks_system_keyboard;
            self.tracks_system_keyboard = configuration.tracks_system_keyboard;
            if was_tracking == self.tracks_system_keyboard
                && composer_bottom_inset_changed
                && self.configured_viewport_bottom_inset < previous_configured_bottom_inset
                && !settles_immediately
                && self.hold_composer_release_for_sent_row(window, cx)
            {
                return;
            }
            if was_tracking && !self.tracks_system_keyboard {
                self.keyboard_target_overlap = 0.0;
                if composer_bottom_inset_changed && !self.keyboard_target_is_visible && !settles_immediately {
                    self.animate_configured_composer_transition(window, cx);
                    return;
                }
                self.finish_configured_keyboard_transition(window, cx);
                return;
            } else if !was_tracking && self.tracks_system_keyboard {
                self.keyboard_target_overlap = self.current_keyboard_guide_overlap();
                if self.keyboard_target_overlap > 0.5
                    || self.configured_viewport_bottom_inset + 0.5 < self.configured_keyboard_resting_bottom_inset
                {
                    self.keyboard_target_is_visible = true;
                }
                if composer_bottom_inset_changed && self.keyboard_target_overlap <= 0.5 && !settles_immediately {
                    self.animate_configured_composer_transition(window, cx);
                    return;
                }
            } else if !self.tracks_system_keyboard && composer_bottom_inset_changed && !settles_immediately {
                self.animate_configured_composer_transition(window, cx);
                return;
            }
        }

        if self.list_view.read(cx).has_active_keyboard_viewport_transition() {
            self.has_pending_configured_viewport_insets = true;
            self.retarget_active_keyboard_transition_if_needed(window, cx);
            return;
        }
        self.has_pending_configured_viewport_insets = false;
        if self.composer_release_awaits_sent_row {
            if !composer_bottom_inset_changed || self.held_composer_release_would_lower_rows(cx) {
                return;
            }
            self.composer_release_awaits_sent_row = false;
        }
        let target = self.resolved_configured_viewport_insets(self.keyboard_target_overlap);
        let list_view = self.list_view.read(cx);
        if list_view.viewport_insets() == target.0 && list_view.bottom_sticky_viewport_inset() == target.1 {
            return;
        }
        if self.tracks_system_keyboard
            && composer_bottom_inset_changed
            && !settles_immediately
            && ChatComposerInsetAnimation::announces_animated_change()
        {
            self.animate_viewport_transition_to_composer_timing(target, window, cx);
            return;
        }
        self.finish_viewport_transition(target, window, cx);
    }

    pub(in super::super) fn retarget_active_keyboard_transition_if_needed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.tracks_system_keyboard {
            return;
        }
        let Some(current) = self.list_view.read(cx).keyboard_viewport_transition_target() else {
            return;
        };
        let target = self.resolved_configured_viewport_insets(self.keyboard_target_overlap);
        if current == target {
            return;
        }
        self.animate_viewport_transition_to_composer_timing(target, window, cx);
    }
}
