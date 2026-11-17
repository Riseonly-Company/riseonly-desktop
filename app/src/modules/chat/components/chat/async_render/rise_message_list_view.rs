use std::cell::Cell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::hash::Hasher as _;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures::StreamExt as _;
use futures::channel::{mpsc, oneshot};
use gpui::{
    AnyElement, AnyView, App, AppContext as _, Bounds, Context, ElementId, Entity,
    Hsla, IntoElement, ParentElement as _, Pixels, Render, RenderOnce, StyleRefinement, Styled as _, Subscription,
    Task, Window, canvas, div, px,
};
use rise_core::foundation::foundation_calendar::{Calendar, TimeZone};
use rise_core::foundation::foundation_date::TimeInterval;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_core::rise_app_events::{RiseAppEvents, RiseNotificationName, RiseSubscription, UIApplication};
use rise_energy::rise_energy_saving::RISE_ENERGY_SAVING_DID_CHANGE;
use rise_i18n::language_store::{LanguageStore, Locale};
use rise_platform::haptics::haptics::{self, ImpactStyle};
use rise_platform::process_info::process_info::THERMAL_STATE_DID_CHANGE;
use rise_theme::theme::theme_palette::ThemePalette;
use rise_theme::theme::theme_store::ThemeStore;
use rise_ui::animated_media_playback_scope::{
    AnimatedMediaPlaybackScopeProviding,
    is_animated_media_playback_active,
};
use rise_ui::swiftui::environment::{LayoutDirection, LayoutDirectionKey, env, env_scope, layout_direction};
use rise_ui::swiftui::environment_values::{ColorScheme, ColorSchemeKey, color_scheme};
use rise_ui::swiftui::lifecycle::LifecycleModifiers as _;
use rise_ui::swiftui::open_url::{OpenURLAction, OpenURLKey, open_url};

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey};
use crate::core::media::rise_media_foreground_gate::{RISE_MEDIA_FOREGROUND_DID_CHANGE, RiseMediaForegroundGate};
use crate::core::media_viewer::media_viewer_presentation_state::{
    MediaViewerPresentationKey, MediaViewerPresentationState, chat_media_presentation,
};
use crate::core::ui::formatted_text_ui::FormattedTextInlinePreparation;
use crate::modules::chat::components::chat::media_viewer::chat_media_gallery_builder::ChatViewLimitedOpenings;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuContainer;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::HoldContextMenuAlignment;
use crate::core::widgets::wrappers::list_shell_ui::list_shell_floating_top_inset;
use crate::modules::chat::components::chat::async_render::async_text_bubble_types::{
    AsyncTextBubbleCorners, AsyncTextBubbleLayoutSpec, AsyncTextBubblePreparedLayout, AsyncTextBubbleSeed,
    AsyncTextBubbleVisualStyle,
};
use crate::modules::chat::components::chat::async_render::chat_message_render_snapshot::{
    ChatMessageRenderEntry, ChatMessageRenderEntryGroupFlags, ChatMessageRenderEntryKind, ChatMessageRenderRequestKey, ChatMessageRenderSnapshot, ChatMessageRenderSnapshotPreparer,
};
use crate::modules::chat::components::chat::async_render::chat_performance_trace::ChatPerformanceTrace;
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_types::{
    RiseManualListItem, RiseManualListItemMountPolicy, RiseManualListScrollMetrics, RiseManualListTransaction,
    RiseManualListTransactionAnchor, RiseManualListTransactionEdgeMutation,
    RiseManualListTransactionUpdateAnimation,
};
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_view::{
    RiseManualListMountEnvelopePolicy, RiseManualListView,
    RiseManualListViewPlaceholderView,
};
use crate::modules::chat::components::chat::async_render::rise_manual_message_hosting_view::RiseManualMessageHostingView;
use crate::modules::chat::components::chat::async_render::rise_message_fallback_height_cache::RiseMessageFallbackHeightCacheKey;
use crate::modules::chat::components::chat::async_render::rise_native_system_row_view::RiseNativeSystemRowView;
use crate::modules::chat::components::chat::bubble::message_content::ChatAllowsReplyJumpKey;
use crate::modules::chat::components::chat::media_viewer::chat_media_presentation::{ChatGalleryMessagesKey, ChatGalleryPaginatesKey};
use crate::modules::chat::components::chat::media_viewer::location_fullscreen::{
    LocationPresentationKey, LocationPresentationState, location_presentation,
};
use crate::modules::chat::components::chat::message_list::MessageList;
use crate::modules::chat::components::chat::message_row::MessageRow;
use crate::modules::chat::components::chat::message_system_pill::{
    MessageSystemPill, MessageTopicDivider,
};
use crate::modules::chat::components::chat::video_note::video_note_player_store::VideoNotePlayerStore;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::scroll::chat_scroll_position_store::{
    ChatScrollPositionStore, ChatScrollPositionStorePosition,
};
use crate::modules::chat::engine::core::rise_conversation_presentation::RiseMessagePresentationContentStamp;
use crate::modules::chat::engine::messages::rise_message_scope::RiseMessagePageDirection;
use crate::modules::chat::engine::messages::rise_message_unread_marks::{RiseUnreadMarkCandidate, RiseUnreadMarkViewport};
use crate::modules::chat::shared::chat_saved_message_presentation::{
    ChatQuickReactionPolicy, ChatSavedMessagePresentation, RiseMessageSenderIdentity,
};
use crate::modules::chat::shared::chat_search_highlight::ChatMessageSearchHighlight;
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::chat::shared::message_permissions::MessagePermissions;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType, User};
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageSystemKind, RemoteMediaURL,
};
use crate::modules::chat::stores::message::message_services::message_presentation_window_projector::{
    MessagePresentationSourceOrder, MessagePresentationWindowProjector, MessagePresentationWindowState,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiseMessageInitialScrollPosition {
    Top,
    Bottom,
}

pub enum RiseMessageFirstFramePresentationPolicy {}

impl RiseMessageFirstFramePresentationPolicy {
    pub fn starts_new_presentation(
        current_epoch: Option<Uuid>,
        incoming_epoch: Option<Uuid>,
        changes_chat: bool,
    ) -> bool {
        changes_chat || current_epoch != incoming_epoch
    }

    pub fn should_supersede_unrevealed_owner(
        owner_source_version: i64,
        incoming_source_version: i64,
        incoming_data_resolved: bool,
    ) -> bool {
        incoming_data_resolved && owner_source_version != incoming_source_version
    }

    pub fn pins_initial_position_to_bottom(
        initial_scroll_position: RiseMessageInitialScrollPosition,
        has_saved_position: bool,
    ) -> bool {
        initial_scroll_position == RiseMessageInitialScrollPosition::Bottom && !has_saved_position
    }
}

pub enum RiseMessageScrollPositionPersistencePolicy {}

impl RiseMessageScrollPositionPersistencePolicy {
    pub const EXACT_BOTTOM_TOLERANCE: f32 = 2.0;

    pub fn discards_saved_position(distance_from_bottom: f32, can_load_newer: bool) -> bool {
        let distance = distance_from_bottom.max(0.0);
        distance <= Self::EXACT_BOTTOM_TOLERANCE
            || (!can_load_newer && distance <= ChatScrollPositionStore::NEAR_BOTTOM_THRESHOLD)
    }
}

/// two producers write anchor_id, the list writes a render entry id and chat search a bare server id
/// lookup resolves against the entry table, the value as written first
pub enum RiseMessageScrollAnchorResolution {}

impl RiseMessageScrollAnchorResolution {
    pub const ENTRY_PREFIXES: [&'static str; 3] = ["msg-", "sys-", "topic-"];

    pub fn candidates(anchor_id: &str) -> Vec<String> {
        if anchor_id.is_empty() {
            return Vec::new();
        }
        if Self::ENTRY_PREFIXES
            .iter()
            .any(|prefix| anchor_id.starts_with(prefix))
            || anchor_id.starts_with("date-")
        {
            return vec![anchor_id.to_string()];
        }
        let mut candidates = Vec::with_capacity(Self::ENTRY_PREFIXES.len() + 1);
        candidates.push(anchor_id.to_string());
        candidates.extend(
            Self::ENTRY_PREFIXES
                .iter()
                .map(|prefix| format!("{prefix}{anchor_id}")),
        );
        candidates
    }
}

pub enum RiseMessageInitialViewportRecoveryPolicy {}

impl RiseMessageInitialViewportRecoveryPolicy {
    pub const MAXIMUM_STABLE_SNAPSHOT_RETRIES: i64 = 2;

    pub fn should_reveal_applied_geometry(after_failed_attempt: i64) -> bool {
        after_failed_attempt >= Self::MAXIMUM_STABLE_SNAPSHOT_RETRIES
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiseMessageInitialFrameRevealDeliveryPolicyAction {
    Deliver,
    DeferUntilWindowAttachment,
    DiscardStale,
}

pub enum RiseMessageInitialFrameRevealDeliveryPolicy {}

impl RiseMessageInitialFrameRevealDeliveryPolicy {
    pub fn action(
        is_current: bool,
        is_attached_to_window: bool,
    ) -> RiseMessageInitialFrameRevealDeliveryPolicyAction {
        if !is_current {
            return RiseMessageInitialFrameRevealDeliveryPolicyAction::DiscardStale;
        }
        if is_attached_to_window {
            RiseMessageInitialFrameRevealDeliveryPolicyAction::Deliver
        } else {
            RiseMessageInitialFrameRevealDeliveryPolicyAction::DeferUntilWindowAttachment
        }
    }
}

pub enum RiseFallbackMeasurementPolicy {}

impl RiseFallbackMeasurementPolicy {
    pub const NEARBY_ENTRY_LIMIT: i64 = 48;
    pub const MINIMUM_GUARD_ENTRY_COUNT: i64 = 8;

    pub fn initial_preparation_range(entry_count: i64) -> Range<i64> {
        0..entry_count.max(0)
    }

    pub fn guarded_visible_range(visible_range: Range<i64>, entry_count: i64) -> Range<i64> {
        if entry_count <= 0 || visible_range.is_empty() {
            return 0..0;
        }
        let lower_bound =
            (entry_count.min(visible_range.start) - Self::MINIMUM_GUARD_ENTRY_COUNT).max(0);
        let upper_bound = entry_count
            .min(entry_count.min(visible_range.end).max(0) + Self::MINIMUM_GUARD_ENTRY_COUNT);
        lower_bound..lower_bound.max(upper_bound)
    }

    pub fn exact_geometry_range_for_commit(
        entry_count: i64,
        prepared_range: Range<i64>,
        is_initial_frame_ready: bool,
        is_deferred_preparation: bool,
    ) -> Range<i64> {
        let full_range = 0..entry_count.max(0);
        if !is_deferred_preparation && is_initial_frame_ready {
            return full_range;
        }
        let lower_bound = entry_count.min(prepared_range.start).max(0);
        let upper_bound = lower_bound.max(entry_count.min(prepared_range.end));
        lower_bound..upper_bound
    }

    pub fn nearby_range(available_range: Range<i64>, centered_at: i64) -> Range<i64> {
        if available_range.is_empty() {
            return available_range;
        }
        let limit = Self::NEARBY_ENTRY_LIMIT.min(available_range.end - available_range.start);
        let center = available_range
            .start
            .max(centered_at)
            .min(available_range.end - 1);
        let lower_bound = available_range
            .start
            .max(center - limit / 2)
            .min(available_range.end - limit);
        lower_bound..(lower_bound + limit)
    }
}

pub enum RiseMessageActiveScrollFallbackMeasurementPolicy {}

impl RiseMessageActiveScrollFallbackMeasurementPolicy {
    pub const MINIMUM_ACTOR_INTERVAL: TimeInterval = 1.0 / 60.0;

    /// one display frame of the screen the list is really on
    /// the minimum actor interval is a 60hz frame, on a 120hz panel that spaces measuring twice too far
    pub fn actor_interval(maximum_frames_per_second: i64) -> TimeInterval {
        1.0 / maximum_frames_per_second.clamp(60, 240) as f64
    }

    pub fn unresolved_critical_indices(
        critical_indices: &[i64],
        unresolved_indices: &[i64],
    ) -> Vec<i64> {
        let unresolved: HashSet<i64> = unresolved_indices.iter().copied().collect();
        let mut visited = HashSet::new();
        critical_indices
            .iter()
            .copied()
            .filter(|index| unresolved.contains(index) && visited.insert(*index))
            .collect()
    }

    pub fn delay_before_next_measurement(
        elapsed_since_previous_measurement: Option<TimeInterval>,
        actor_interval: TimeInterval,
    ) -> TimeInterval {
        match elapsed_since_previous_measurement {
            Some(elapsed) => (actor_interval - elapsed).max(0.0),
            None => 0.0,
        }
    }
}

pub enum RiseMessagePreparedGeometryPolicy {}

impl RiseMessagePreparedGeometryPolicy {
    pub fn identities_can_form_scroll_safe_mutation(
        old_ids: &[String],
        new_ids: &[String],
    ) -> bool {
        let is_unique = |ids: &[String]| {
            let unique: HashSet<&String> = ids.iter().collect();
            unique.len() == ids.len()
        };
        is_unique(old_ids) && is_unique(new_ids)
    }

    pub fn has_exact_fallback_height(height: Option<f32>) -> bool {
        height.is_some_and(|height| height.is_finite() && height > 0.0)
    }

    pub fn can_reuse_retained_item_geometry(
        previous_version: Option<i64>,
        next_version: i64,
        retained_item_height: Option<f32>,
    ) -> bool {
        previous_version == Some(next_version)
            && Self::has_exact_fallback_height(retained_item_height)
    }

    pub fn should_adopt_previous_fallback_height(
        previous_version: Option<i64>,
        next_version: i64,
        candidate_height: Option<f32>,
        previous_height: Option<f32>,
    ) -> bool {
        previous_version == Some(next_version)
            && !Self::has_exact_fallback_height(candidate_height)
            && Self::has_exact_fallback_height(previous_height)
    }
}

pub enum RiseMessageActiveScrollEdgeCommitPolicy {}

impl RiseMessageActiveScrollEdgeCommitPolicy {
    pub fn allows_commit(
        is_tracking: bool,
        is_dragging: bool,
        is_decelerating: bool,
        has_stable_edge_mutation: bool,
        visible_items_are_retained: bool,
        exact_geometry_is_prepared: bool,
    ) -> bool {
        (is_tracking || is_dragging || is_decelerating)
            && has_stable_edge_mutation
            && visible_items_are_retained
            && exact_geometry_is_prepared
    }

    pub fn visible_items_survive_mutation(
        visible_ids: &HashSet<String>,
        retained_ids: &HashSet<String>,
        removable_decoration_ids: &HashSet<String>,
    ) -> bool {
        !visible_ids.is_disjoint(retained_ids)
            && visible_ids
                .difference(retained_ids)
                .all(|id| removable_decoration_ids.contains(id))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RiseMessageScrollDestination {
    Takeover,
    Latest,
    Message(String),
    DayStart(i64),
}

pub enum RiseMessageScrollLandingPolicy {}

impl RiseMessageScrollLandingPolicy {
    pub fn lands_in_commit(
        destination: &RiseMessageScrollDestination,
        can_load_newer: bool,
    ) -> bool {
        match destination {
            RiseMessageScrollDestination::Takeover => false,
            RiseMessageScrollDestination::Latest => !can_load_newer,
            RiseMessageScrollDestination::Message(_) | RiseMessageScrollDestination::DayStart(_) => {
                true
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseMessageScrollRequest {
    pub id: Uuid,
    pub destination: RiseMessageScrollDestination,
    pub animated: bool,
}

impl RiseMessageScrollRequest {
    pub fn new(destination: RiseMessageScrollDestination, animated: bool) -> Self {
        Self {
            id: Uuid::new_v4(),
            destination,
            animated,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RiseMessageViewportState {
    pub is_at_latest: bool,
    pub shows_jump_to_latest: bool,
}

pub enum RiseMessageKeyboardViewportPolicy {}

impl RiseMessageKeyboardViewportPolicy {
    pub fn resolved_inset(
        configured_inset: f32,
        resting_inset: f32,
        keyboard_overlap: f32,
        tracks_system_keyboard: bool,
        keyboard_active_inset: Option<f32>,
    ) -> f32 {
        let configured = configured_inset.max(0.0);
        if !tracks_system_keyboard {
            return configured;
        }
        let overlap = keyboard_overlap.max(0.0);
        if overlap <= 0.5 {
            return configured.max(resting_inset);
        }
        let base = keyboard_active_inset.unwrap_or(configured).max(0.0);
        configured.max(base + overlap)
    }
}

pub enum RiseMessageKeyboardRestingInsetPolicy {}

impl RiseMessageKeyboardRestingInsetPolicy {
    pub fn resolved_inset(
        configured_inset: f32,
        keyboard_resting_inset: f32,
        keyboard_overlap: f32,
        keyboard_is_visible: bool,
    ) -> f32 {
        if keyboard_is_visible
            && keyboard_overlap <= 0.5
            && configured_inset + 0.5 < keyboard_resting_inset
        {
            return configured_inset;
        }
        keyboard_resting_inset
    }
}

pub enum RiseMessageViewportPolicy {}

impl RiseMessageViewportPolicy {
    pub const LATEST_THRESHOLD: f32 = 100.0;
    pub const SHOW_THRESHOLD: f32 = 180.0;

    pub fn resolve(
        distance_from_bottom: f32,
        can_load_newer: bool,
        previous: Option<RiseMessageViewportState>,
    ) -> RiseMessageViewportState {
        let distance = distance_from_bottom.max(0.0);
        let is_at_latest = !can_load_newer && distance <= Self::LATEST_THRESHOLD;
        let threshold = if previous.is_some_and(|state| state.shows_jump_to_latest) {
            Self::LATEST_THRESHOLD
        } else {
            Self::SHOW_THRESHOLD
        };
        RiseMessageViewportState {
            is_at_latest,
            shows_jump_to_latest: !is_at_latest && (can_load_newer || distance > threshold),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RiseMessageHistoryEdgeThresholds {
    pub load: f32,
    pub rearm: f32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseMessagePendingPresentationCommit {
    pub account_user_id: Option<String>,
    pub account_epoch: Option<String>,
    pub chat_id: String,
    pub presentation_epoch: Option<Uuid>,
    pub source_version: i64,
}

impl RiseMessagePendingPresentationCommit {
    pub fn belongs_to_presentation(
        &self,
        account_user_id: Option<&str>,
        account_epoch: Option<&str>,
        chat_id: &str,
        presentation_epoch: Option<Uuid>,
    ) -> bool {
        self.account_user_id.as_deref() == account_user_id
            && self.account_epoch.as_deref() == account_epoch
            && self.chat_id == chat_id
            && self.presentation_epoch == presentation_epoch
    }

    pub fn matches_committed_presentation(
        &self,
        account_user_id: Option<&str>,
        account_epoch: Option<&str>,
        chat_id: &str,
        presentation_epoch: Option<Uuid>,
        source_version: i64,
    ) -> bool {
        self.belongs_to_presentation(account_user_id, account_epoch, chat_id, presentation_epoch)
            && self.source_version == source_version
    }
}

pub enum RiseMessagePendingPresentationCommitPolicy {}

impl RiseMessagePendingPresentationCommitPolicy {
    pub fn should_track_incoming(
        current: Option<&RiseMessagePendingPresentationCommit>,
        incoming: &RiseMessagePendingPresentationCommit,
        starts_new_presentation: bool,
        changes_account: bool,
    ) -> bool {
        starts_new_presentation
            || changes_account
            || current.is_some_and(|current| {
                current.belongs_to_presentation(
                    incoming.account_user_id.as_deref(),
                    incoming.account_epoch.as_deref(),
                    &incoming.chat_id,
                    incoming.presentation_epoch,
                )
            })
    }

    pub fn should_clear_after_commit(
        pending: Option<&RiseMessagePendingPresentationCommit>,
        committed: &RiseMessagePendingPresentationCommit,
        did_commit: bool,
    ) -> bool {
        did_commit
            && pending.is_some_and(|pending| {
                pending.matches_committed_presentation(
                    committed.account_user_id.as_deref(),
                    committed.account_epoch.as_deref(),
                    &committed.chat_id,
                    committed.presentation_epoch,
                    committed.source_version,
                )
            })
    }
}

pub enum RiseMessageEdgePagePacingPolicy {}

impl RiseMessageEdgePagePacingPolicy {
    pub const FAST_SPEED: f32 = 1_000.0;
    pub const RUNWAY_SECONDS: f32 = 0.6;

    pub fn holds(speed: f32, distance_to_edge: f32, viewport_height: f32) -> bool {
        if speed <= Self::FAST_SPEED {
            return false;
        }
        let runway = viewport_height.max(1.0) + speed * Self::RUNWAY_SECONDS;
        distance_to_edge > runway
    }
}

pub enum RiseMessageHistoryEdgePolicy {}

impl RiseMessageHistoryEdgePolicy {
    pub const CACHED_ENTRY_LOAD_THRESHOLD: i64 = 56;
    pub const MINIMUM_REQUEST_INTERVAL: TimeInterval = 0.35;

    pub const PARKED_CANDIDATE_RETRY_INTERVAL: TimeInterval = 0.12;

    pub fn cached_entry_rearm_threshold() -> i64 {
        MessagePresentationWindowProjector::stride()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn can_evaluate_prefetch(
        has_restored_initial_position: bool,
        has_entries: bool,
        has_delivered_initial_frame: bool,
        has_observed_user_interaction: bool,
        viewport_is_underfilled: bool,
        has_pending_presentation_commit: bool,
        snapshot_apply_in_flight: bool,
        is_tracking: bool,
        is_dragging: bool,
        is_decelerating: bool,
    ) -> bool {
        let has_active_motion = is_tracking || is_dragging || is_decelerating;
        let can_fill_settled_underflow =
            has_delivered_initial_frame && viewport_is_underfilled && !snapshot_apply_in_flight;
        has_restored_initial_position
            && has_entries
            && !has_pending_presentation_commit
            && (has_active_motion
                || (has_delivered_initial_frame && has_observed_user_interaction)
                || can_fill_settled_underflow)
    }

    pub fn thresholds(viewport_height: f32, toward_velocity: f32) -> RiseMessageHistoryEdgeThresholds {
        let viewport_height = viewport_height.max(1.0);
        let base = (viewport_height * 3.0).max(1_200.0);
        let lookahead = (viewport_height * 5.0).min(toward_velocity.max(0.0) * 0.7);
        let load = base + lookahead;
        RiseMessageHistoryEdgeThresholds {
            load,
            rearm: (load + (viewport_height * 0.75).max(600.0)).max(viewport_height * 2.75),
        }
    }

    /// heads_toward_edge means the list moves toward this edge or came to rest after moving toward it
    pub fn is_inside_load_zone(
        distance: f32,
        load_threshold: f32,
        has_cached_page: bool,
        cached_entry_distance: Option<i64>,
        heads_toward_edge: bool,
    ) -> bool {
        if distance <= load_threshold {
            return true;
        }
        match cached_entry_distance {
            Some(cached_entry_distance) if heads_toward_edge && has_cached_page => {
                cached_entry_distance <= Self::CACHED_ENTRY_LOAD_THRESHOLD
            }
            _ => false,
        }
    }

    /// edge the last motion headed for, rest keeps it and a new presentation clears it
    pub fn heads_toward_edge(is_older_edge: bool, velocity_y: f32, last_motion_toward_older: Option<bool>) -> bool {
        if velocity_y < 0.0 {
            return is_older_edge;
        }
        if velocity_y > 0.0 {
            return !is_older_edge;
        }
        last_motion_toward_older == Some(is_older_edge)
    }

    pub fn is_outside_rearm_zone(
        distance: f32,
        rearm_threshold: f32,
        has_cached_page: bool,
        cached_entry_distance: Option<i64>,
    ) -> bool {
        if distance <= rearm_threshold {
            return false;
        }
        match cached_entry_distance {
            Some(cached_entry_distance) if has_cached_page => {
                cached_entry_distance > Self::cached_entry_rearm_threshold()
            }
            _ => true,
        }
    }

    pub fn can_request(
        is_armed: bool,
        has_pending_request: bool,
        can_load: bool,
        has_callback: bool,
        elapsed_since_last_request: TimeInterval,
    ) -> bool {
        is_armed
            && !has_pending_request
            && can_load
            && has_callback
            && elapsed_since_last_request >= Self::MINIMUM_REQUEST_INTERVAL
    }

    pub fn did_availability_change(
        previous_can_load: bool,
        can_load: bool,
        previous_has_cached_page: bool,
        has_cached_page: bool,
    ) -> bool {
        previous_can_load != can_load || previous_has_cached_page != has_cached_page
    }

    pub fn should_rearm_after_history_progress(
        did_make_progress_at_requested_edge: bool,
        can_load: bool,
    ) -> bool {
        did_make_progress_at_requested_edge && can_load
    }

    pub fn did_complete_pending_request(
        did_mutate_requested_edge: bool,
        availability_changed: bool,
    ) -> bool {
        did_mutate_requested_edge || availability_changed
    }

    pub fn commit_belongs_to_pending_request(
        pending_generation: u64,
        committed_generation: Option<u64>,
    ) -> bool {
        committed_generation == Some(pending_generation)
    }

    pub fn pending_watchdog_delay(has_cached_page: bool) -> TimeInterval {
        if has_cached_page { 12.0 } else { 8.0 }
    }

    pub fn rendered_window_is_current(
        requested_source_version: Option<i64>,
        applied_source_version: Option<i64>,
        has_candidate_in_flight: bool,
    ) -> bool {
        requested_source_version == applied_source_version || !has_candidate_in_flight
    }

    pub fn request_timestamp_after_cancellation(
        existing_timestamp: TimeInterval,
        callback_was_invoked: bool,
    ) -> TimeInterval {
        if callback_was_invoked {
            existing_timestamp
        } else {
            0.0
        }
    }

    pub fn should_reevaluate_after_cancellation(
        callback_was_invoked: bool,
        observed_progress: bool,
    ) -> bool {
        observed_progress || !callback_was_invoked
    }

    pub fn deliver_request_if_current(is_current: bool, perform: impl FnOnce()) -> bool {
        if !is_current {
            return false;
        }
        perform();
        true
    }
}

pub enum RiseMessageSnapshotReusePolicy {}

impl RiseMessageSnapshotReusePolicy {
    #[allow(clippy::too_many_arguments)]
    pub fn can_reuse(
        cached_chat_id: &str,
        cached_source_version: i64,
        cached_group_by_topic: bool,
        cached_topic_titles: &HashMap<String, String>,
        chat_id: &str,
        source_version: i64,
        group_by_topic: bool,
        topic_titles: &HashMap<String, String>,
    ) -> bool {
        if cached_chat_id != chat_id
            || cached_source_version != source_version
            || cached_group_by_topic != group_by_topic
        {
            return false;
        }
        !group_by_topic || cached_topic_titles == topic_titles
    }

    pub fn has_equivalent_rendered_content(
        current: &ChatMessageRenderSnapshot,
        next: &ChatMessageRenderSnapshot,
    ) -> bool {
        if current.entries.len() != next.entries.len() {
            return false;
        }
        current
            .entries
            .iter()
            .zip(next.entries.iter())
            .all(|(current, next)| {
                current.id == next.id
                    && current.kind == next.kind
                    && current.content_version == next.content_version
            })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RiseMessageFeedSourceVersionPolicyDataVersion {
    Bounded {
        content_revision: i64,
        generation: u64,
        source_order: MessagePresentationSourceOrder,
        canonical_count: i64,
        lower_bound: i64,
        upper_bound: i64,
        anchor_id: Option<String>,
        anchor_offset: i64,
    },
    Cache(i64),
}

pub enum RiseMessageFeedSourceVersionPolicy {}

impl RiseMessageFeedSourceVersionPolicy {
    pub fn data_version(
        cache_revision: i64,
        presentation_state: Option<&MessagePresentationWindowState>,
    ) -> RiseMessageFeedSourceVersionPolicyDataVersion {
        match presentation_state {
            Some(presentation_state) => RiseMessageFeedSourceVersionPolicyDataVersion::Bounded {
                content_revision: cache_revision,
                generation: presentation_state.generation,
                source_order: presentation_state.source_order,
                canonical_count: presentation_state.canonical_count,
                lower_bound: presentation_state.range.start,
                upper_bound: presentation_state.range.end,
                anchor_id: presentation_state.anchor_id.clone(),
                anchor_offset: presentation_state.anchor_offset,
            },
            None => RiseMessageFeedSourceVersionPolicyDataVersion::Cache(cache_revision),
        }
    }
}

pub enum RiseMessageNativeLayoutReusePolicy {}

impl RiseMessageNativeLayoutReusePolicy {
    #[allow(clippy::too_many_arguments)]
    pub fn can_reuse(
        entry_content_version: i64,
        cached_content_version: Option<i64>,
        cached_spec: &AsyncTextBubbleLayoutSpec,
        proposed_spec: &AsyncTextBubbleLayoutSpec,
        cached_time_zone_identifier: &str,
        proposed_time_zone_identifier: &str,
        cached_ownership: Option<bool>,
        proposed_ownership: bool,
    ) -> bool {
        cached_content_version == Some(entry_content_version)
            && cached_spec == proposed_spec
            && cached_time_zone_identifier == proposed_time_zone_identifier
            && cached_ownership == Some(proposed_ownership)
    }
}

pub enum RiseMessageRuntimeHeightCorrectionPolicy {}

impl RiseMessageRuntimeHeightCorrectionPolicy {
    /// a correction is held back only while the user owns the list
    /// corrections ride the update transition and slide so applying them to visible rows is fine
    pub fn can_apply(
        initial_frame_ready: bool,
        is_user_scrolling: bool,
        _visible_ids: &HashSet<String>,
        _correction_ids: &HashSet<String>,
    ) -> bool {
        if !initial_frame_ready {
            return true;
        }
        !is_user_scrolling
    }
}

pub enum RiseMessageAnimatedMediaScrollPolicy {}

impl RiseMessageAnimatedMediaScrollPolicy {
    pub fn should_suspend_playback(
        _is_currently_suspended: bool,
        _metrics: &RiseManualListScrollMetrics,
    ) -> bool {
        false
    }
}

pub type RiseMessageListViewStringPairHandler = Rc<dyn Fn(&str, &str, &mut Window, &mut App)>;
pub type RiseMessageListViewOpenSenderProfileHandler = Rc<dyn Fn(&User, &mut Window, &mut App)>;
pub type RiseMessageListViewSelectTopicHandler = Rc<dyn Fn(Option<&str>, &mut Window, &mut App)>;
pub type RiseMessageListViewSelectDateHandler = Rc<dyn Fn(i64, bool, &mut Window, &mut App)>;
pub type RiseMessageListViewLoadPageHandler = Rc<dyn Fn(Option<&str>, &mut Window, &mut App)>;
pub type RiseMessageListViewPrefetchPageHandler =
    Rc<dyn Fn(Option<RiseMessagePageDirection>, &mut Window, &mut App)>;
pub type RiseMessageListViewVoidHandler = Rc<dyn Fn(&mut Window, &mut App)>;
pub type RiseMessageListViewBoolHandler = Rc<dyn Fn(bool, &mut Window, &mut App)>;
pub type RiseMessageListViewViewportStateHandler =
    Rc<dyn Fn(&RiseMessageViewportState, &mut Window, &mut App)>;
pub type RiseMessageListViewMarkCandidatesHandler =
    Rc<dyn Fn(RiseUnreadMarkViewport, &mut Window, &mut App)>;
pub type RiseMessageListViewToggleSelectionHandler =
    Rc<dyn Fn(&GetMessageMessage, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct RiseMessageListView {
    pub(super) id: ElementId,
    pub messages: Vec<Arc<GetMessageMessage>>,
    pub chat: Arc<ChatInfo>,
    pub chat_id: String,
    pub bottom_inset: f32,
    pub sticky_bottom_inset: Option<f32>,
    pub keyboard_resting_bottom_inset: Option<f32>,
    pub keyboard_resting_sticky_bottom_inset: Option<f32>,
    pub keyboard_active_bottom_inset: Option<f32>,
    pub keyboard_active_sticky_bottom_inset: Option<f32>,
    pub tracks_system_keyboard: bool,
    pub settles_composer_inset_immediately: bool,
    pub source_version: i64,
    pub content_stamps_by_local_id: HashMap<String, RiseMessagePresentationContentStamp>,
    pub presentation_epoch: Option<Uuid>,
    pub initial_scroll_position: RiseMessageInitialScrollPosition,
    pub scroll_request: Option<RiseMessageScrollRequest>,
    pub scroll_position_key: Option<String>,
    pub persists_scroll_position: bool,
    pub interactions_enabled: bool,
    pub allows_message_selection: bool,
    pub allows_reply_jump: bool,
    pub gallery_paginates: bool,
    pub message_selection_active: bool,
    pub selected_message_ids: HashSet<String>,
    pub on_toggle_message_selection: Option<RiseMessageListViewToggleSelectionHandler>,
    pub initial_data_resolved: bool,
    pub can_load_older: bool,
    pub can_load_newer: bool,
    pub has_cached_older: bool,
    pub has_cached_newer: bool,
    pub group_by_topic: bool,
    pub topic_titles: HashMap<String, String>,
    pub on_reaction: Option<RiseMessageListViewStringPairHandler>,
    pub on_action: Option<RiseMessageListViewStringPairHandler>,
    pub on_open_sender_profile: Option<RiseMessageListViewOpenSenderProfileHandler>,
    pub on_select_topic: Option<RiseMessageListViewSelectTopicHandler>,
    pub on_select_date: Option<RiseMessageListViewSelectDateHandler>,
    pub on_load_older: Option<RiseMessageListViewLoadPageHandler>,
    pub on_load_newer: Option<RiseMessageListViewLoadPageHandler>,
    pub on_prefetch_page: Option<RiseMessageListViewPrefetchPageHandler>,
    pub on_initial_frame_ready: Option<RiseMessageListViewVoidHandler>,
    pub on_scroll_activity_changed: Option<RiseMessageListViewBoolHandler>,
    pub on_viewport_state_changed: Option<RiseMessageListViewViewportStateHandler>,
    pub on_visible_mark_candidates_changed: Option<RiseMessageListViewMarkCandidatesHandler>,
    pub search_highlight: Option<ChatMessageSearchHighlight>,
    pub unread_separator_message_id: Option<String>,
    pub view_limited_closed: HashSet<String>,
    pub is_scroll_enabled: bool,
}

/// environment values of the message list view, read while it renders
/// the snapshot completion can run on a later turn and must use what this render saw
pub(super) struct RiseMessageListViewEnvironment {
    pub(super) theme_store: Entity<ThemeStore>,
    pub(super) navigation_store: Option<Entity<NavigationStore>>,
    pub(super) open_url: OpenURLAction,
    pub(super) media_presentation: Entity<MediaViewerPresentationState>,
    pub(super) location_presentation: Option<Entity<LocationPresentationState>>,
    pub(super) shell_top_inset: f32,
    pub(super) locale: Locale,
    pub(super) calendar: Calendar,
    pub(super) time_zone: TimeZone,
    pub(super) layout_direction: LayoutDirection,
    pub(super) color_scheme: ColorScheme,
    pub(super) is_animated_media_playback_active: bool,
}

pub(super) fn current_user_id(cx: &mut App) -> Option<String> {
    AuthServicesStore::shared(cx).read(cx).profile().map(|profile| profile.id.clone())
}

impl RiseMessageListView {
    pub fn new(
        id: impl Into<ElementId>,
        messages: Vec<Arc<GetMessageMessage>>,
        chat: Arc<ChatInfo>,
        chat_id: impl Into<String>,
        bottom_inset: f32,
        source_version: i64,
    ) -> Self {
        Self {
            id: id.into(),
            messages,
            chat,
            chat_id: chat_id.into(),
            bottom_inset,
            sticky_bottom_inset: None,
            keyboard_resting_bottom_inset: None,
            keyboard_resting_sticky_bottom_inset: None,
            keyboard_active_bottom_inset: None,
            keyboard_active_sticky_bottom_inset: None,
            tracks_system_keyboard: false,
            settles_composer_inset_immediately: false,
            source_version,
            content_stamps_by_local_id: HashMap::new(),
            presentation_epoch: None,
            initial_scroll_position: RiseMessageInitialScrollPosition::Bottom,
            scroll_request: None,
            scroll_position_key: None,
            persists_scroll_position: true,
            interactions_enabled: true,
            allows_message_selection: false,
            allows_reply_jump: false,
            gallery_paginates: true,
            message_selection_active: false,
            selected_message_ids: HashSet::new(),
            on_toggle_message_selection: None,
            initial_data_resolved: true,
            can_load_older: false,
            can_load_newer: false,
            has_cached_older: false,
            has_cached_newer: false,
            group_by_topic: false,
            topic_titles: HashMap::new(),
            on_reaction: None,
            on_action: None,
            on_open_sender_profile: None,
            on_select_topic: None,
            on_select_date: None,
            on_load_older: None,
            on_load_newer: None,
            on_prefetch_page: None,
            on_initial_frame_ready: None,
            on_scroll_activity_changed: None,
            on_viewport_state_changed: None,
            on_visible_mark_candidates_changed: None,
            search_highlight: None,
            unread_separator_message_id: None,
            view_limited_closed: HashSet::new(),
            is_scroll_enabled: true,
        }
    }

    pub(super) fn read_environment(cx: &mut App) -> RiseMessageListViewEnvironment {
        RiseMessageListViewEnvironment {
            theme_store: ThemeStore::shared(cx),
            navigation_store: env::<NavigationStoreKey>(cx),
            open_url: open_url(cx),
            media_presentation: chat_media_presentation(cx),
            location_presentation: location_presentation(cx),
            shell_top_inset: list_shell_floating_top_inset(cx),
            locale: LanguageStore::current_locale(),
            calendar: Calendar::current(),
            time_zone: TimeZone::current(),
            layout_direction: layout_direction(cx),
            color_scheme: color_scheme(cx),
            is_animated_media_playback_active: is_animated_media_playback_active(cx),
        }
    }

    pub(super) fn update_ui_view(
        self: Rc<Self>,
        environment: Rc<RiseMessageListViewEnvironment>,
        ui_view: &Entity<RiseMessageManualListView>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let (view_limited_opened, view_limited_closed) = {
            let openings = ChatViewLimitedOpenings::shared(cx).read(cx);
            let closed: HashSet<String> = self.view_limited_closed.union(openings.closed()).cloned().collect();
            (openings.opened().clone(), closed)
        };
        ui_view.update(cx, |ui_view, cx| {
            ui_view.set_animated_media_playback_active(
                environment.is_animated_media_playback_active,
                window,
                cx,
            );
            let is_inside_view_update = Rc::new(Cell::new(true));
            let completion_flag = is_inside_view_update.clone();
            let this = self.clone();
            let completion_environment = environment.clone();
            let completion: RiseMessageManualListViewRenderCompletion =
                Box::new(move |snapshot, ui_view, window, cx| {
                    if !completion_flag.get() || !ui_view.has_rendered_configuration() {
                        this.apply_prepared_snapshot(
                            snapshot,
                            &completion_environment,
                            ui_view,
                            window,
                            cx,
                        );
                        return;
                    }
                    cx.defer_in(window, move |ui_view, window, cx| {
                        this.apply_prepared_snapshot(
                            snapshot,
                            &completion_environment,
                            ui_view,
                            window,
                            cx,
                        );
                    });
                });
            ui_view.request_render_snapshot(
                self.messages.clone(),
                &self.chat_id,
                self.source_version,
                self.content_stamps_by_local_id.clone(),
                self.group_by_topic,
                self.topic_titles.clone(),
                environment.locale.clone(),
                environment.calendar.clone(),
                environment.time_zone.clone(),
                self.presentation_epoch,
                ChatSavedMessagePresentation::is_saved_messages_chat(&self.chat),
                self.search_highlight.clone(),
                self.unread_separator_message_id.clone(),
                view_limited_opened,
                view_limited_closed,
                completion,
                window,
                cx,
            );
            is_inside_view_update.set(false);
        });
    }

    pub(super) fn apply_prepared_snapshot(
        &self,
        snapshot: Arc<ChatMessageRenderSnapshot>,
        environment: &RiseMessageListViewEnvironment,
        ui_view: &mut RiseMessageManualListView,
        window: &mut Window,
        cx: &mut Context<RiseMessageManualListView>,
    ) {
        let apply_trace = ChatPerformanceTrace::shared().begin_operation(
            "snapshot.apply_configuration",
            HashMap::from([
                ("entries".to_owned(), snapshot.entries.len().to_string()),
                ("source".to_owned(), self.source_version.to_string()),
            ]),
        );
        let (theme_palette, tint_color, text_delta) = {
            let theme_store = environment.theme_store.read(cx);
            (
                theme_store.current_palette().clone(),
                theme_store.current_theme().primary._100,
                theme_store.text_delta(),
            )
        };
        ui_view.apply(
            RiseMessageCollectionConfiguration {
                snapshot: snapshot.clone(),
                messages: Arc::new(self.messages.clone()),
                chat: self.chat.clone(),
                chat_id: self.chat_id.clone(),
                presentation_epoch: self.presentation_epoch,
                scroll_position_key: self.scroll_position_key.clone(),
                top_inset: environment.shell_top_inset,
                bottom_inset: self.bottom_inset,
                sticky_bottom_inset: self.sticky_bottom_inset,
                keyboard_resting_bottom_inset: self.keyboard_resting_bottom_inset,
                keyboard_resting_sticky_bottom_inset: self.keyboard_resting_sticky_bottom_inset,
                keyboard_active_bottom_inset: self.keyboard_active_bottom_inset,
                keyboard_active_sticky_bottom_inset: self.keyboard_active_sticky_bottom_inset,
                tracks_system_keyboard: self.tracks_system_keyboard,
                settles_composer_inset_immediately: self.settles_composer_inset_immediately,
                initial_scroll_position: self.initial_scroll_position,
                scroll_request: self.scroll_request.clone(),
                persists_scroll_position: self.persists_scroll_position,
                interactions_enabled: self.interactions_enabled,
                allows_message_selection: self.allows_message_selection,
                allows_reply_jump: self.allows_reply_jump,
                gallery_paginates: self.gallery_paginates,
                initial_data_resolved: self.initial_data_resolved,
                can_load_older: self.can_load_older,
                can_load_newer: self.can_load_newer,
                has_cached_older: self.has_cached_older,
                has_cached_newer: self.has_cached_newer,
                is_scroll_enabled: self.is_scroll_enabled,
                theme_store: environment.theme_store.clone(),
                theme_palette,
                navigation_store: environment.navigation_store.clone(),
                open_url: environment.open_url.clone(),
                media_presentation: environment.media_presentation.clone(),
                location_presentation: environment.location_presentation.clone(),
                locale: environment.locale.clone(),
                calendar: environment.calendar.clone(),
                time_zone: environment.time_zone.clone(),
                layout_direction: environment.layout_direction,
                color_scheme: environment.color_scheme,
                text_delta,
                quick_reaction: Rc::new(ChatQuickReactionPolicy::default_raw),
                on_reaction: self.on_reaction.clone(),
                on_action: self.on_action.clone(),
                on_open_sender_profile: self.on_open_sender_profile.clone(),
                on_select_topic: self.on_select_topic.clone(),
                on_select_date: self.on_select_date.clone(),
                on_load_older: self.on_load_older.clone(),
                on_load_newer: self.on_load_newer.clone(),
                on_initial_frame_ready: self.on_initial_frame_ready.clone(),
                on_scroll_activity_changed: self.on_scroll_activity_changed.clone(),
                on_viewport_state_changed: self.on_viewport_state_changed.clone(),
                on_visible_mark_candidates_changed: self.on_visible_mark_candidates_changed.clone(),
                search_highlight: self.search_highlight.clone(),
                on_prefetch_page: self.on_prefetch_page.clone(),
            },
            window,
            cx,
        );
        ui_view.apply_selection_presentation(
            &snapshot,
            &self.chat,
            self.allows_message_selection,
            self.message_selection_active,
            &self.selected_message_ids,
            tint_color,
            self.on_toggle_message_selection.clone(),
            window,
            cx,
        );
        ChatPerformanceTrace::shared().end_operation(apply_trace, 1.0, HashMap::new());
    }
}

impl RenderOnce for RiseMessageListView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = self.id.clone();
        let ui_view = window.use_keyed_state(id.clone(), cx, |window, cx| {
            RiseMessageManualListView::new(window, cx)
        });
        let environment = Rc::new(Self::read_environment(cx));
        Rc::new(self).update_ui_view(environment, &ui_view, window, cx);
        let appeared = ui_view.clone();
        let dismantled = ui_view.clone();
        div()
            .size_full()
            .child(AnyView::from(ui_view).cached(StyleRefinement::default().size_full()))
            .on_appear(ElementId::NamedChild(Arc::new(id.clone()), "appear".into()), move |window, cx| {
                appeared.update(cx, |ui_view, cx| ui_view.did_move_to_window(true, window, cx));
            })
            .on_disappear(ElementId::NamedChild(Arc::new(id), "dismantle".into()), move |window, cx| {
                dismantled.update(cx, |ui_view, cx| match window {
                    Some(window) => {
                        ui_view.did_move_to_window(false, window, cx);
                        ui_view.stop(window, cx);
                    }
                    None => ui_view.stop_after_window_closed(cx),
                });
            })
    }
}

#[derive(Clone)]
pub struct RiseMessageCollectionConfiguration {
    pub snapshot: Arc<ChatMessageRenderSnapshot>,
    pub messages: Arc<Vec<Arc<GetMessageMessage>>>,
    pub chat: Arc<ChatInfo>,
    pub chat_id: String,
    pub presentation_epoch: Option<Uuid>,
    pub scroll_position_key: Option<String>,
    pub top_inset: f32,
    pub bottom_inset: f32,
    pub sticky_bottom_inset: Option<f32>,
    pub keyboard_resting_bottom_inset: Option<f32>,
    pub keyboard_resting_sticky_bottom_inset: Option<f32>,
    pub keyboard_active_bottom_inset: Option<f32>,
    pub keyboard_active_sticky_bottom_inset: Option<f32>,
    pub tracks_system_keyboard: bool,
    pub settles_composer_inset_immediately: bool,
    pub initial_scroll_position: RiseMessageInitialScrollPosition,
    pub scroll_request: Option<RiseMessageScrollRequest>,
    pub persists_scroll_position: bool,
    pub interactions_enabled: bool,
    pub allows_message_selection: bool,
    pub allows_reply_jump: bool,
    pub gallery_paginates: bool,
    pub initial_data_resolved: bool,
    pub can_load_older: bool,
    pub can_load_newer: bool,
    pub has_cached_older: bool,
    pub has_cached_newer: bool,
    pub is_scroll_enabled: bool,
    pub theme_store: Entity<ThemeStore>,
    pub theme_palette: ThemePalette,
    pub navigation_store: Option<Entity<NavigationStore>>,
    pub open_url: OpenURLAction,
    pub media_presentation: Entity<MediaViewerPresentationState>,
    pub location_presentation: Option<Entity<LocationPresentationState>>,
    pub locale: Locale,
    pub calendar: Calendar,
    pub time_zone: TimeZone,
    pub layout_direction: LayoutDirection,
    pub color_scheme: ColorScheme,
    pub text_delta: f32,
    pub quick_reaction: Rc<dyn Fn(&App) -> String>,
    pub on_reaction: Option<RiseMessageListViewStringPairHandler>,
    pub on_action: Option<RiseMessageListViewStringPairHandler>,
    pub on_open_sender_profile: Option<RiseMessageListViewOpenSenderProfileHandler>,
    pub on_select_topic: Option<RiseMessageListViewSelectTopicHandler>,
    pub on_select_date: Option<RiseMessageListViewSelectDateHandler>,
    pub on_load_older: Option<RiseMessageListViewLoadPageHandler>,
    pub on_load_newer: Option<RiseMessageListViewLoadPageHandler>,
    pub on_initial_frame_ready: Option<RiseMessageListViewVoidHandler>,
    pub on_scroll_activity_changed: Option<RiseMessageListViewBoolHandler>,
    pub on_viewport_state_changed: Option<RiseMessageListViewViewportStateHandler>,
    pub on_visible_mark_candidates_changed: Option<RiseMessageListViewMarkCandidatesHandler>,
    pub search_highlight: Option<ChatMessageSearchHighlight>,
    pub on_prefetch_page: Option<RiseMessageListViewPrefetchPageHandler>,
}

impl RiseMessageCollectionConfiguration {
    pub fn can_trigger_quick_reaction(&self, message: &GetMessageMessage, cx: &mut App) -> bool {
        self.resolved_quick_reaction(message, cx).is_some()
    }

    /// key the reading position is stored under, falls back to the message scope so non forum
    /// screens keep their old id
    pub fn scroll_storage_key(&self) -> String {
        let key = self
            .scroll_position_key
            .as_deref()
            .map(trimming_whitespaces_and_newlines)
            .unwrap_or("");
        if key.is_empty() {
            self.chat_id.clone()
        } else {
            key.to_owned()
        }
    }

    /// chat search writes its jump anchor under the bare chat id, it cant know which tab a forum opens on
    /// a screen with a topic qualified key reads that anchor too
    pub fn scroll_storage_fallback_key(&self) -> Option<String> {
        if self.scroll_storage_key() == self.chat_id {
            None
        } else {
            Some(self.chat_id.clone())
        }
    }

    pub fn saved_scroll_position(&self, cx: &mut App) -> Option<ChatScrollPositionStorePosition> {
        if let Some(position) = ChatScrollPositionStore::saved_position(&self.scroll_storage_key(), cx) {
            return Some(position);
        }
        let fallback_key = self.scroll_storage_fallback_key()?;
        ChatScrollPositionStore::saved_position(&fallback_key, cx)
    }

    pub fn trigger_quick_reaction(&self, message: &GetMessageMessage, window: &mut Window, cx: &mut App) {
        let Some(reaction) = self.resolved_quick_reaction(message, cx) else {
            return;
        };
        let _ = haptics::impact(ImpactStyle::Light);
        if let Some(on_reaction) = &self.on_reaction {
            on_reaction(&message.id, &reaction, window, cx);
        }
    }

    pub(super) fn resolved_quick_reaction(&self, message: &GetMessageMessage, cx: &mut App) -> Option<String> {
        if !self.interactions_enabled || self.on_reaction.is_none() {
            return None;
        }
        let raw = (self.quick_reaction)(cx);
        ChatQuickReactionPolicy::resolved(message, &self.chat, &raw, cx)
    }
}

#[derive(IntoElement)]
pub struct RiseMessageCollectionCellRoot {
    pub entry: Rc<ChatMessageRenderEntry>,
    pub configuration: Rc<RiseMessageCollectionConfiguration>,
    pub prepared_text: Option<FormattedTextInlinePreparation>,
}

impl RiseMessageCollectionCellRoot {
    pub fn new(
        entry: Rc<ChatMessageRenderEntry>,
        configuration: Rc<RiseMessageCollectionConfiguration>,
        prepared_text: Option<FormattedTextInlinePreparation>,
    ) -> Self {
        Self { entry, configuration, prepared_text }
    }

    pub(super) fn content(&self, cx: &mut App) -> AnyElement {
        let entry = &self.entry;
        let configuration = &self.configuration;
        match entry.kind {
            ChatMessageRenderEntryKind::DateSeparator | ChatMessageRenderEntryKind::UnreadSeparator => {
                div()
                    .pt(px(6.0))
                    .pb(px(2.0))
                    .child(MessageSystemPill::new(entry.label.clone()))
                    .into_any_element()
            }
            ChatMessageRenderEntryKind::TopicSeparator => {
                let on_select_topic = configuration.on_select_topic.clone();
                let topic_id = entry.topic_id.clone();
                div()
                    .pt(px(6.0))
                    .pb(px(2.0))
                    .child(MessageTopicDivider::new(
                        entry.label.clone(),
                        Rc::new(move |window: &mut Window, cx: &mut App| {
                            if let Some(on_select_topic) = &on_select_topic {
                                on_select_topic(topic_id.as_deref(), window, cx);
                            }
                        }),
                    ))
                    .into_any_element()
            }
            ChatMessageRenderEntryKind::System => self.system_row(cx),
            ChatMessageRenderEntryKind::Message => match (&entry.message, entry.group) {
                (Some(message), Some(group)) => self.message_row(message.clone(), group, cx),
                _ => div().into_any_element(),
            },
        }
    }

    pub(super) fn system_pill(entry: &ChatMessageRenderEntry) -> MessageSystemPill {
        let system = entry.message.as_ref().and_then(|message| message.system.as_ref());
        MessageSystemPill::new(entry.label.clone())
            .wallpaper_preview(system.and_then(|system| system.wallpaper_preview.clone()))
            .shows_gift(system.is_some_and(|system| system.kind == MessageSystemKind::PremiumGifted))
            .gift_item(RiseNativeSystemRowView::gift_item(entry).cloned())
    }

    pub(super) fn system_row(&self, cx: &mut App) -> AnyElement {
        let entry = self.entry.clone();
        let configuration = &self.configuration;
        if let Some(message) = entry.message.clone().filter(|_| configuration.interactions_enabled) {
            let actions = MessagePermissions::new(&message, &configuration.chat, false).context_menu_actions(
                configuration.allows_message_selection,
                false,
                cx,
            );
            let on_action = configuration.on_action.clone();
            let message_id = message.id.clone();
            let source_entry = entry.clone();
            return HoldContextMenuContainer::new(
                ElementId::Name(format!("system-row-{}", entry.id).into()),
                HoldContextMenuAlignment::Leading,
                Rc::new(move |_cx: &App| actions.clone()),
                Rc::new(move |_window: &mut Window, _cx: &mut App| {
                    div()
                        .py(px(2.0))
                        .child(Self::system_pill(&source_entry))
                        .into_any_element()
                }),
            )
            .show_emoji_list(false)
            .uses_flat_chrome(true)
            .on_action(Rc::new(move |action: String, window: &mut Window, cx: &mut App| {
                if let Some(on_action) = &on_action {
                    on_action(&message_id, &action, window, cx);
                }
            }))
            .into_any_element();
        }
        div()
            .py(px(2.0))
            .child(Self::system_pill(&entry))
            .into_any_element()
    }

    pub(super) fn message_row(
        &self,
        message: Arc<GetMessageMessage>,
        group: ChatMessageRenderEntryGroupFlags,
        cx: &mut App,
    ) -> AnyElement {
        let configuration = self.configuration.clone();
        let is_own = MessageList::is_own_bubble(&message, &configuration.chat, cx);
        let show_avatar = configuration.chat.r#type == ChatType::Group
            || configuration.chat.r#type == ChatType::Favourites;

        let reaction_configuration = configuration.clone();
        let reaction_id = message.id.clone();
        let action_configuration = configuration.clone();
        let action_id = message.id.clone();
        let profile_configuration = configuration.clone();
        let profile_message = message.clone();
        let can_double_tap = configuration.can_trigger_quick_reaction(&message, cx);
        let tap_configuration = configuration.clone();
        let tap_message = message.clone();

        let row = MessageRow::new(
            message,
            configuration.chat.clone(),
            is_own,
            group.is_first,
            group.is_last,
            show_avatar,
        )
        .allows_reply(false)
        .interactions_enabled(configuration.interactions_enabled)
        .allows_message_selection(configuration.allows_message_selection)
        .on_reaction(Some(Rc::new(move |emoji: &str, window: &mut Window, cx: &mut App| {
            if let Some(on_reaction) = &reaction_configuration.on_reaction {
                on_reaction(&reaction_id, emoji, window, cx);
            }
        })))
        .on_action(Some(Rc::new(move |action: &str, window: &mut Window, cx: &mut App| {
            if let Some(on_action) = &action_configuration.on_action {
                on_action(&action_id, action, window, cx);
            }
        })))
        .on_open_sender_profile(Some(Rc::new(move |window: &mut Window, cx: &mut App| {
            if let Some(on_open) = &profile_configuration.on_open_sender_profile {
                let participant = RiseMessageSenderIdentity::participant(
                    &profile_message,
                    &profile_configuration.chat,
                    cx,
                );
                on_open(&participant, window, cx);
            }
        })))
        .can_double_tap(Rc::new(move || can_double_tap))
        .on_double_tap(Some(Rc::new(move |window: &mut Window, cx: &mut App| {
            tap_configuration.trigger_quick_reaction(&tap_message, window, cx);
        })))
        .prepared_text(self.prepared_text.clone());

        div()
            .pt(px(if group.is_first {
                MessageSizes::list_first_in_group_top_padding()
            } else {
                MessageSizes::list_continuation_top_padding()
            }))
            .pb(px(if group.is_last {
                MessageSizes::list_last_in_group_bottom_padding()
            } else {
                0.0
            }))
            .child(row)
            .into_any_element()
    }
}

impl RenderOnce for RiseMessageCollectionCellRoot {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let content = div().w_full().child(self.content(cx));
        let configuration = &self.configuration;
        let content = env_scope::<ColorSchemeKey>(Some(configuration.color_scheme)).child(content);
        let content = env_scope::<LayoutDirectionKey>(configuration.layout_direction).child(content);
        let content = env_scope::<ChatAllowsReplyJumpKey>(configuration.allows_reply_jump).child(content);
        let content = env_scope::<ChatGalleryPaginatesKey>(configuration.gallery_paginates).child(content);
        let content =
            env_scope::<ChatGalleryMessagesKey>(Some(configuration.messages.clone())).child(content);
        let content =
            env_scope::<LocationPresentationKey>(configuration.location_presentation.clone()).child(content);
        let content = env_scope::<MediaViewerPresentationKey>(Some(configuration.media_presentation.clone()))
            .child(content);
        let content = env_scope::<OpenURLKey>(configuration.open_url.clone()).child(content);
        env_scope::<NavigationStoreKey>(configuration.navigation_store.clone()).child(content)
    }
}

pub(super) const RISE_MESSAGE_MANUAL_LIST_VIEW_DEFERRED_HISTORY_CHUNK_SIZE: usize = 48;
pub(super) const RISE_MESSAGE_MANUAL_LIST_VIEW_DEFERRED_WORK_FLUSH_PASS_LIMIT: usize = 8;

pub(super) struct RiseMessageManualListViewSnapshotCacheValue {
    pub(super) key: ChatMessageRenderRequestKey,
    pub(super) snapshot: Arc<ChatMessageRenderSnapshot>,
}

#[derive(Clone)]
pub(super) struct RiseMessageManualListViewNativePreparationRequest {
    pub(super) id: String,
    pub(super) content_version: i64,
    pub(super) seed: AsyncTextBubbleSeed,
    pub(super) spec: AsyncTextBubbleLayoutSpec,
}

pub(super) struct RiseMessageManualListViewNativePredisplayInput {
    pub(super) prepared_layout: Arc<AsyncTextBubblePreparedLayout>,
    pub(super) style: AsyncTextBubbleVisualStyle,
    pub(super) corners: AsyncTextBubbleCorners,
    pub(super) scale: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RiseMessageManualListViewNativePreparationPurpose {
    Foreground,
    DeferredHistory,
}

#[derive(Clone)]
pub(super) struct RiseMessageManualListViewPreparedNativeBatch {
    pub(super) chat_id: String,
    pub(super) source_version: i64,
    pub(super) configuration_revision: u64,
    pub(super) width: f32,
    pub(super) layouts: HashMap<String, Arc<AsyncTextBubblePreparedLayout>>,
    pub(super) versions: HashMap<String, i64>,
    pub(super) ownership_by_id: HashMap<String, bool>,
    pub(super) entry_range: Range<usize>,
    pub(super) purpose: RiseMessageManualListViewNativePreparationPurpose,
    pub(super) hosted_text: HashMap<String, FormattedTextInlinePreparation>,
}

#[derive(Clone)]
pub(super) struct RiseMessageManualListViewPreparedSnapshotCandidate {
    pub(super) configuration: Rc<RiseMessageCollectionConfiguration>,
    pub(super) configuration_revision: u64,
    pub(super) native_batch: RiseMessageManualListViewPreparedNativeBatch,
    pub(super) fallback_heights: HashMap<String, f32>,
    pub(super) history_edge_load_generation: Option<u64>,
}

pub(super) struct RiseMessageManualListViewScrollSafeEdgeCommit {
    pub(super) mutation: RiseManualListTransactionEdgeMutation,
    pub(super) retained_ids: HashSet<String>,
    pub(super) removable_visible_ids: HashSet<String>,
}

pub(super) struct RiseMessageManualListViewPreparedManualItemTable {
    pub(super) items_by_id: Rc<HashMap<String, RiseManualListItem>>,
    pub(super) updated_ids: HashSet<String>,
}

pub(super) struct RiseMessageManualListViewRuntimeHeightCorrection {
    pub(super) entry_content_version: i64,
    pub(super) source_version: i64,
    pub(super) width: f32,
    pub(super) cache_key: RiseMessageFallbackHeightCacheKey,
    pub(super) height: f32,
}

#[derive(Clone)]
pub(super) struct RiseMessageManualListViewStickyAvatarGroupContent {
    pub(super) id: String,
    pub(super) participant: User,
}

pub(super) struct RiseMessageManualListViewDeferredMarkCandidate {
    pub(super) entry_id: String,
    pub(super) candidate: RiseUnreadMarkCandidate,
}

#[derive(Clone, PartialEq)]
pub(super) struct RiseMessageManualListViewStickyAvatarGroupCacheKey {
    pub(super) chat_id: String,
    pub(super) source_version: i64,
    pub(super) entry_count: usize,
    pub(super) first_entry_id: Option<String>,
    pub(super) last_entry_id: Option<String>,
    pub(super) chat_type: ChatType,
    pub(super) current_user_id: String,
}

#[derive(Clone)]
pub(super) struct RiseMessageManualListViewInitialFrameOwner {
    pub(super) chat_id: String,
    pub(super) source_version: i64,
    pub(super) entry_count: usize,
    pub(super) pins_initial_position_to_bottom: bool,
}

pub(super) struct RiseMessageManualListViewPendingInitialFrameReveal {
    pub(super) readiness_generation: u64,
    pub(super) apply_generation: u64,
    pub(super) configuration_revision: u64,
    pub(super) chat_id: String,
    pub(super) source_version: i64,
    pub(super) presentation_epoch: Option<Uuid>,
    pub(super) viewport_ids: Vec<String>,
    pub(super) ready_configuration: Rc<RiseMessageCollectionConfiguration>,
    pub(super) queued_configuration: Option<Rc<RiseMessageCollectionConfiguration>>,
    pub(super) callback: Option<RiseMessageListViewVoidHandler>,
}

#[derive(Clone, PartialEq)]
pub(super) struct RiseMessageManualListViewDeferredHistoryPreparationKey {
    pub(super) chat_id: String,
    pub(super) source_version: i64,
    pub(super) configuration_revision: u64,
    pub(super) width: f32,
}

pub(super) enum RiseMessageManualListViewFallbackMeasurementResult {
    Completed(RiseMessageManualListViewPreparedSnapshotCandidate),
    Paused(RiseMessageManualListViewPreparedSnapshotCandidate),
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RiseMessageManualListViewHistoryEdge {
    Older,
    Newer,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RiseMessageManualListViewPendingHistoryEdgeLoad {
    pub(super) edge: RiseMessageManualListViewHistoryEdge,
    pub(super) chat_id: String,
    pub(super) source_version: i64,
    pub(super) generation: u64,
    pub(super) is_cached_shift: bool,
}

pub(super) type RiseMessageManualListViewRenderCompletion = Box<
    dyn FnOnce(
        Arc<ChatMessageRenderSnapshot>,
        &mut RiseMessageManualListView,
        &mut Window,
        &mut Context<RiseMessageManualListView>,
    ),
>;

pub(super) type RiseMessageManualListViewNativeCompletion = Box<
    dyn FnOnce(
        RiseMessageManualListViewPreparedNativeBatch,
        &mut RiseMessageManualListView,
        &mut Window,
        &mut Context<RiseMessageManualListView>,
    ),
>;

#[derive(Clone, Copy)]
pub(super) enum RiseMessageManualListViewSignal {
    RefreshSpeculativePreparation,
    ApplicationDidEnterBackground,
    ApplicationDidBecomeActive,
}

pub struct RiseMessageManualListView {
    pub(super) list_view: Entity<RiseManualListView>,
    pub(super) list_alpha: f32,
    pub(super) is_in_window: bool,
    pub(super) window_is_active: bool,
    pub(super) display_scale: f32,
    pub(super) meaningful_activity_source: Uuid,
    pub(super) last_active_scroll_metrics_handled: Option<RiseManualListScrollMetrics>,
    pub(super) bounds: Bounds<Pixels>,
    pub(super) render_snapshot_preparer: Arc<ChatMessageRenderSnapshotPreparer>,
    pub(super) snapshot_cache: Option<RiseMessageManualListViewSnapshotCacheValue>,
    pub(super) render_preparation_task: Option<Task<()>>,
    pub(super) render_preparation_cancelled: Option<Arc<AtomicBool>>,
    pub(super) render_preparation_generation: u64,
    pub(super) pending_render_key: Option<ChatMessageRenderRequestKey>,
    pub(super) pending_render_completion: Option<RiseMessageManualListViewRenderCompletion>,
    pub(super) configuration: Option<Rc<RiseMessageCollectionConfiguration>>,
    pub(super) configuration_revision: u64,
    pub(super) item_environment_revision: u64,
    pub(super) presentation_revision: u64,
    pub(super) rendered_configuration: Option<Rc<RiseMessageCollectionConfiguration>>,
    pub(super) measurement_scratch_view: Option<Entity<RiseManualMessageHostingView>>,
    pub(super) rendered_configuration_revision: u64,
    pub(super) last_requested_theme_palette: Option<ThemePalette>,
    pub(super) pending_prepared_candidate: Option<RiseMessageManualListViewPreparedSnapshotCandidate>,
    pub(super) parked_candidate_retry_scheduled: bool,
    pub(super) entries_by_id: HashMap<String, Arc<ChatMessageRenderEntry>>,
    pub(super) versions_by_id: HashMap<String, i64>,
    pub(super) rendered_native_ids: HashSet<String>,
    pub(super) last_reported_mark_candidate_ids: HashSet<String>,
    pub(super) deferred_mark_candidates: HashMap<String, RiseMessageManualListViewDeferredMarkCandidate>,
    pub(super) deferred_mark_chat_id: Option<String>,
    pub(super) last_mark_scroll_movement_at: TimeInterval,
    pub(super) last_mark_scroll_offset_y: Option<Pixels>,
    pub(super) deferred_mark_hold_still_work: Option<Task<()>>,
    pub(super) applied_source_version: Option<i64>,
    pub(super) requested_source_version: Option<i64>,
    pub(super) did_restore_initial_position: bool,
    pub(super) load_older_armed: bool,
    pub(super) last_load_older_at: TimeInterval,
    pub(super) load_newer_armed: bool,
    pub(super) last_load_newer_at: TimeInterval,
    pub(super) last_history_motion_toward_older: Option<bool>,
    pub(super) pending_history_edge_load: Option<RiseMessageManualListViewPendingHistoryEdgeLoad>,
    pub(super) pending_history_edge_progress_generation: Option<u64>,
    pub(super) pending_presentation_commit: Option<RiseMessagePendingPresentationCommit>,
    pub(super) remote_prefetch_direction: Option<RiseMessagePageDirection>,
    pub(super) remote_prefetch_source_version: Option<i64>,
    pub(super) history_edge_load_generation_by_source_version: HashMap<i64, u64>,
    pub(super) history_edge_load_generation: u64,
    pub(super) native_layouts: HashMap<String, Arc<AsyncTextBubblePreparedLayout>>,
    pub(super) hosted_text: HashMap<String, FormattedTextInlinePreparation>,
    pub(super) native_layout_versions: HashMap<String, i64>,
    pub(super) native_ownership_by_id: HashMap<String, bool>,
    pub(super) native_preparation_generation: u64,
    pub(super) native_preparation_task: Option<Task<()>>,
    pub(super) pending_native_configuration: Option<Rc<RiseMessageCollectionConfiguration>>,
    pub(super) pending_native_completion: Option<RiseMessageManualListViewNativeCompletion>,
    pub(super) pending_native_entry_range: Option<Range<usize>>,
    pub(super) pending_native_purpose: RiseMessageManualListViewNativePreparationPurpose,
    pub(super) prepared_snapshot_finalization_task: Option<Task<()>>,
    pub(super) prepared_snapshot_finalization_generation: u64,
    pub(super) candidate_being_predisplayed: Option<RiseMessageManualListViewPreparedSnapshotCandidate>,
    pub(super) visible_predisplay_task: Option<Task<()>>,
    pub(super) last_visible_predisplay_center: Option<usize>,
    pub(super) media_prefetch_scope: String,
    pub(super) media_prefetch_generation: u64,
    pub(super) media_prefetch_account_user_id: Option<String>,
    pub(super) media_prefetch_account_epoch: Option<String>,
    pub(super) last_media_prefetch_center: Option<usize>,
    pub(super) last_media_prefetch_direction_is_newer: Option<bool>,
    pub(super) last_media_prefetch_source_version: Option<i64>,
    pub(super) apply_generation: u64,
    pub(super) is_applying_snapshot: bool,
    pub(super) native_preparation_width: f32,
    pub(super) rendered_preparation_width: f32,
    pub(super) measured_heights: HashMap<String, f32>,
    pub(super) measured_height_width: f32,
    pub(super) grown_video_note_message_id: Option<String>,
    pub(super) pending_runtime_height_corrections: HashMap<String, RiseMessageManualListViewRuntimeHeightCorrection>,
    pub(super) runtime_height_correction_flush_scheduled: bool,
    pub(super) manual_items_by_id: Rc<HashMap<String, RiseManualListItem>>,
    pub(super) manual_items_width: f32,
    pub(super) sticky_avatar_group_cache: Option<(
        RiseMessageManualListViewStickyAvatarGroupCacheKey,
        HashMap<String, RiseMessageManualListViewStickyAvatarGroupContent>,
    )>,
    pub(super) pending_width_repreparation: bool,
    pub(super) viewport_resize_anchor: Option<RiseManualListTransactionAnchor>,
    pub(super) pending_scroll_reset: bool,
    pub(super) pending_fallback_measurement: Option<RiseMessageManualListViewFallbackMeasurementRequest>,
    pub(super) pending_scroll_request: Option<RiseMessageScrollRequest>,
    pub(super) pending_reveal_highlight: Option<(String, Hsla)>,
    pub(super) is_flushing_deferred_work: bool,
    pub(super) needs_another_deferred_work_flush: bool,
    pub(super) was_user_scrolling: bool,
    pub(super) has_observed_history_scroll_interaction: bool,
    pub(super) published_viewport_state: Option<RiseMessageViewportState>,
    pub(super) pending_viewport_state: Option<RiseMessageViewportState>,
    pub(super) viewport_state_delivery_scheduled: bool,
    pub(super) viewport_state_delivery_generation: u64,
    pub(super) presentation_chat_id: Option<String>,
    pub(super) presentation_epoch: Option<Uuid>,
    pub(super) initial_frame_ready_delivered: bool,
    pub(super) initial_viewport_readiness_generation: u64,
    pub(super) initial_viewport_readiness_pass_in_flight: bool,
    pub(super) initial_viewport_height_correction_pass: i64,
    pub(super) initial_viewport_readiness_watchdog: Option<Task<()>>,
    pub(super) initial_frame_owner: Option<RiseMessageManualListViewInitialFrameOwner>,
    pub(super) unrendered_outgoing_ids: HashSet<String>,
    pub(super) maintains_initial_bottom_pin: bool,
    pub(super) queued_pre_reveal_configuration: Option<Rc<RiseMessageCollectionConfiguration>>,
    pub(super) pending_initial_frame_reveal: Option<RiseMessageManualListViewPendingInitialFrameReveal>,
    pub(super) rendered_foreground_preparation_range: Range<usize>,
    pub(super) deferred_history_preparation_key: Option<RiseMessageManualListViewDeferredHistoryPreparationKey>,
    pub(super) deferred_history_prepared_indices: BTreeSet<usize>,
    pub(super) deferred_history_visible_range: Option<Range<usize>>,
    pub(super) deferred_history_last_visible_center: Option<usize>,
    pub(super) deferred_history_moves_toward_newer: bool,
    pub(super) deferred_history_needs_initial_guard: bool,
    pub(super) active_deferred_history_range: Option<Range<usize>>,
    pub(super) tracks_system_keyboard: bool,
    pub(super) configured_viewport_top_inset: f32,
    pub(super) configured_viewport_bottom_inset: f32,
    pub(super) configured_keyboard_resting_bottom_inset: f32,
    pub(super) configured_keyboard_active_bottom_inset: Option<f32>,
    pub(super) configured_sticky_bottom_inset: Option<f32>,
    pub(super) configured_keyboard_resting_sticky_bottom_inset: Option<f32>,
    pub(super) configured_keyboard_active_sticky_bottom_inset: Option<f32>,
    pub(super) keyboard_target_overlap: f32,
    pub(super) keyboard_target_is_visible: bool,
    pub(super) has_pending_configured_viewport_insets: bool,
    pub(super) composer_release_awaits_sent_row: bool,
    pub(super) composer_release_generation: u64,
    pub(super) composer_release_held_keyboard_overlap: f32,
    pub(super) requested_animated_media_playback_active: bool,
    pub(super) suspends_animated_media_for_fast_scroll: bool,
    pub(super) is_animated_media_playback_active: bool,
    pub(super) preparation_environment_observers: Vec<RiseSubscription>,
    pub(super) _signal_delivery: Task<()>,
    pub(super) _app_quit_subscription: Subscription,
    pub(super) _window_closed_subscription: Subscription,
}

impl AnimatedMediaPlaybackScopeProviding for RiseMessageManualListView {
    fn is_animated_media_playback_active(&self) -> bool {
        self.is_animated_media_playback_active
    }
}

impl RiseMessageManualListView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let _ = RiseMediaForegroundGate::shared(cx);
        let (sender, mut receiver) = mpsc::unbounded::<RiseMessageManualListViewSignal>();
        let center = RiseAppEvents::default_center();
        let mut preparation_environment_observers: Vec<RiseSubscription> = [
            RISE_MEDIA_FOREGROUND_DID_CHANGE,
            RiseNotificationName::new(RISE_ENERGY_SAVING_DID_CHANGE),
            RiseNotificationName::new(THERMAL_STATE_DID_CHANGE),
        ]
        .into_iter()
        .map(|name| {
            let sender = sender.clone();
            center.observe(name, move |_| {
                let _ = sender.unbounded_send(RiseMessageManualListViewSignal::RefreshSpeculativePreparation);
            })
        })
        .collect();
        let background_sender = sender.clone();
        preparation_environment_observers.push(center.observe(
            UIApplication::DID_ENTER_BACKGROUND_NOTIFICATION,
            move |_| {
                let _ = background_sender
                    .unbounded_send(RiseMessageManualListViewSignal::ApplicationDidEnterBackground);
            },
        ));
        preparation_environment_observers.push(center.observe(
            UIApplication::DID_BECOME_ACTIVE_NOTIFICATION,
            move |_| {
                let _ = sender.unbounded_send(RiseMessageManualListViewSignal::ApplicationDidBecomeActive);
            },
        ));
        let signal_delivery = cx.spawn_in(window, async move |this, cx| {
            while let Some(signal) = receiver.next().await {
                let delivered = this.update_in(cx, |this, window, cx| match signal {
                    RiseMessageManualListViewSignal::RefreshSpeculativePreparation => {
                        this.refresh_speculative_preparation(window, cx);
                    }
                    RiseMessageManualListViewSignal::ApplicationDidEnterBackground => {
                        this.application_did_enter_background(window, cx);
                    }
                    RiseMessageManualListViewSignal::ApplicationDidBecomeActive => {
                        this.application_did_become_active(window, cx);
                    }
                });
                if delivered.is_err() {
                    return;
                }
            }
        });

        ChatScrollPositionStore::observe_app_exit(cx);
        let app_quit_subscription = cx.on_app_quit(|this, cx| {
            this.persist_scroll_position(None, cx);
            ChatScrollPositionStore::flush_for_exit(cx)
        });
        let own_window_id = window.window_handle().window_id();
        let weak_this = cx.weak_entity();
        let window_closed_subscription = cx.on_window_closed(move |cx, window_id| {
            if window_id != own_window_id {
                return;
            }
            let _ = weak_this.update(cx, |this, cx| {
                this.persist_scroll_position(None, cx);
                ChatScrollPositionStore::flush_for_exit(cx).detach();
            });
        });

        let list_view = cx.new(|cx| RiseManualListView::new(window, cx));
        list_view.update(cx, |list_view, cx| {
            list_view.set_inter_item_spacing(px(MessageSizes::list_row_spacing()), window, cx);
            list_view.set_reuse_pool_limit(36);
        });
        Self::install_list_callbacks(&list_view, cx);

        let mut this = Self {
            list_view,
            list_alpha: 0.0,
            window_is_active: Self::window_counts_as_active(window),
            display_scale: window.scale_factor(),
            meaningful_activity_source: Uuid::new_v4(),
            last_active_scroll_metrics_handled: None,
            is_in_window: false,
            bounds: Bounds::default(),
            render_snapshot_preparer: Arc::new(ChatMessageRenderSnapshotPreparer::new()),
            snapshot_cache: None,
            render_preparation_task: None,
            render_preparation_cancelled: None,
            render_preparation_generation: 0,
            pending_render_key: None,
            pending_render_completion: None,
            configuration: None,
            configuration_revision: 0,
            item_environment_revision: 0,
            presentation_revision: 0,
            rendered_configuration: None,
            measurement_scratch_view: None,
            rendered_configuration_revision: 0,
            last_requested_theme_palette: None,
            pending_prepared_candidate: None,
            parked_candidate_retry_scheduled: false,
            entries_by_id: HashMap::new(),
            versions_by_id: HashMap::new(),
            rendered_native_ids: HashSet::new(),
            last_reported_mark_candidate_ids: HashSet::new(),
            deferred_mark_candidates: HashMap::new(),
            deferred_mark_chat_id: None,
            last_mark_scroll_movement_at: 0.0,
            last_mark_scroll_offset_y: None,
            deferred_mark_hold_still_work: None,
            applied_source_version: None,
            requested_source_version: None,
            did_restore_initial_position: false,
            load_older_armed: true,
            last_load_older_at: 0.0,
            load_newer_armed: true,
            last_load_newer_at: 0.0,
            last_history_motion_toward_older: None,
            pending_history_edge_load: None,
            pending_history_edge_progress_generation: None,
            pending_presentation_commit: None,
            remote_prefetch_direction: None,
            remote_prefetch_source_version: None,
            history_edge_load_generation_by_source_version: HashMap::new(),
            history_edge_load_generation: 0,
            native_layouts: HashMap::new(),
            hosted_text: HashMap::new(),
            native_layout_versions: HashMap::new(),
            native_ownership_by_id: HashMap::new(),
            native_preparation_generation: 0,
            native_preparation_task: None,
            pending_native_configuration: None,
            pending_native_completion: None,
            pending_native_entry_range: None,
            pending_native_purpose: RiseMessageManualListViewNativePreparationPurpose::Foreground,
            prepared_snapshot_finalization_task: None,
            prepared_snapshot_finalization_generation: 0,
            candidate_being_predisplayed: None,
            visible_predisplay_task: None,
            last_visible_predisplay_center: None,
            media_prefetch_scope: format!("chat-message-list-{}", Uuid::new_v4()),
            media_prefetch_generation: 0,
            media_prefetch_account_user_id: None,
            media_prefetch_account_epoch: None,
            last_media_prefetch_center: None,
            last_media_prefetch_direction_is_newer: None,
            last_media_prefetch_source_version: None,
            apply_generation: 0,
            is_applying_snapshot: false,
            native_preparation_width: 0.0,
            rendered_preparation_width: 0.0,
            measured_heights: HashMap::new(),
            measured_height_width: 0.0,
            grown_video_note_message_id: None,
            pending_runtime_height_corrections: HashMap::new(),
            runtime_height_correction_flush_scheduled: false,
            manual_items_by_id: Rc::new(HashMap::new()),
            manual_items_width: 0.0,
            sticky_avatar_group_cache: None,
            pending_width_repreparation: false,
            viewport_resize_anchor: None,
            pending_scroll_reset: false,
            pending_scroll_request: None,
            pending_reveal_highlight: None,
            pending_fallback_measurement: None,
            is_flushing_deferred_work: false,
            needs_another_deferred_work_flush: false,
            was_user_scrolling: false,
            has_observed_history_scroll_interaction: false,
            published_viewport_state: None,
            pending_viewport_state: None,
            viewport_state_delivery_scheduled: false,
            viewport_state_delivery_generation: 0,
            presentation_chat_id: None,
            presentation_epoch: None,
            initial_frame_ready_delivered: false,
            initial_viewport_readiness_generation: 0,
            initial_viewport_readiness_pass_in_flight: false,
            initial_viewport_height_correction_pass: 0,
            initial_viewport_readiness_watchdog: None,
            initial_frame_owner: None,
            unrendered_outgoing_ids: HashSet::new(),
            maintains_initial_bottom_pin: false,
            queued_pre_reveal_configuration: None,
            pending_initial_frame_reveal: None,
            rendered_foreground_preparation_range: 0..0,
            deferred_history_preparation_key: None,
            deferred_history_prepared_indices: BTreeSet::new(),
            deferred_history_visible_range: None,
            deferred_history_last_visible_center: None,
            deferred_history_moves_toward_newer: false,
            deferred_history_needs_initial_guard: false,
            active_deferred_history_range: None,
            tracks_system_keyboard: false,
            configured_viewport_top_inset: 10.0,
            configured_viewport_bottom_inset: 10.0,
            configured_keyboard_resting_bottom_inset: 10.0,
            configured_keyboard_active_bottom_inset: None,
            configured_sticky_bottom_inset: None,
            configured_keyboard_resting_sticky_bottom_inset: None,
            configured_keyboard_active_sticky_bottom_inset: None,
            keyboard_target_overlap: 0.0,
            keyboard_target_is_visible: false,
            has_pending_configured_viewport_insets: false,
            composer_release_awaits_sent_row: false,
            composer_release_generation: 0,
            composer_release_held_keyboard_overlap: 0.0,
            requested_animated_media_playback_active: true,
            suspends_animated_media_for_fast_scroll: false,
            is_animated_media_playback_active: true,
            preparation_environment_observers,
            _signal_delivery: signal_delivery,
            _app_quit_subscription: app_quit_subscription,
            _window_closed_subscription: window_closed_subscription,
        };
        this.observe_video_note_playback(window, cx);
        cx.observe_window_activation(window, |this, window, cx| {
            this.window_is_active = Self::window_counts_as_active(window);
            this.update_meaningful_activity(cx);
            if this.window_is_active && this.initial_frame_ready_delivered {
                this.last_reported_mark_candidate_ids.clear();
                let visible_ids = this.visible_ids(cx);
                this.report_visible_mark_candidates(&visible_ids, window, cx);
            }
        })
        .detach();
        this
    }

    pub(super) fn install_list_callbacks(list_view: &Entity<RiseManualListView>, cx: &mut Context<Self>) {
        let this = cx.weak_entity();
        list_view.update(cx, |list_view, _| {
            let weak = this.clone();
            list_view.on_keyboard_viewport_transition_did_finish =
                Some(Rc::new(move |window: &mut Window, cx: &mut App| {
                    let weak = weak.clone();
                    window.defer(cx, move |window, cx| {
                        let _ = weak.update(cx, |this, cx| {
                            if !this.has_pending_configured_viewport_insets {
                                return;
                            }
                            this.has_pending_configured_viewport_insets = false;
                            this.apply_configured_viewport_insets(None, window, cx);
                        });
                    });
                }));
            let weak = this.clone();
            list_view.on_visible_ids_changed =
                Some(Rc::new(move |ids: &[String], window: &mut Window, cx: &mut App| {
                    let weak = weak.clone();
                    let ids = ids.to_vec();
                    window.defer(cx, move |window, cx| {
                        let _ = weak.update(cx, |this, cx| this.visible_ids_did_change(&ids, window, cx));
                    });
                }));
            let weak = this.clone();
            list_view.on_readable_ids_changed = Some(Rc::new(move |_: &[String], window: &mut Window, cx: &mut App| {
                let weak = weak.clone();
                window.defer(cx, move |window, cx| {
                    let _ = weak.update(cx, |this, cx| this.readable_ids_did_change(window, cx));
                });
            }));
            let weak = this.clone();
            list_view.on_drag_ended = Some(Rc::new(move |will_decelerate: bool, window: &mut Window, cx: &mut App| {
                if will_decelerate {
                    return;
                }
                let weak = weak.clone();
                window.defer(cx, move |window, cx| {
                    let _ = weak.update(cx, |this, cx| this.flush_deferred_mark_candidates(window, cx));
                });
            }));
            let weak = this.clone();
            list_view.on_active_scroll_metrics_changed = Some(Rc::new(
                move |metrics: &RiseManualListScrollMetrics, window: &mut Window, cx: &mut App| {
                    let weak = weak.clone();
                    let metrics = metrics.clone();
                    window.defer(cx, move |window, cx| {
                        let _ = weak.update(cx, |this, cx| {
                            this.active_scroll_metrics_did_change(&metrics, window, cx);
                        });
                    });
                },
            ));
            let weak = this.clone();
            list_view.on_scroll_metrics_changed = Some(Rc::new(
                move |metrics: &RiseManualListScrollMetrics, window: &mut Window, cx: &mut App| {
                    let weak = weak.clone();
                    let metrics = metrics.clone();
                    window.defer(cx, move |window, cx| {
                        let _ = weak.update(cx, |this, cx| this.scroll_metrics_did_change(&metrics, window, cx));
                    });
                },
            ));
            let weak = this;
            list_view.on_viewport_width_changed =
                Some(Rc::new(move |width: Pixels, window: &mut Window, cx: &mut App| {
                    let weak = weak.clone();
                    window.defer(cx, move |window, cx| {
                        let _ = weak.update(cx, |this, cx| {
                            this.viewport_width_did_change(f32::from(width), window, cx);
                        });
                    });
                }));
        });
    }

    pub fn has_rendered_configuration(&self) -> bool {
        self.rendered_configuration.is_some()
    }

    pub(super) fn layout_subviews(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        if bounds.size.height != self.bounds.size.height {
            self.list_view.update(cx, |list_view, cx| {
                list_view.set_preload_distance(
                    RiseManualListMountEnvelopePolicy::default_preload_distance(bounds.size.height),
                    window,
                    cx,
                );
            });
        }
        let previous_size = self.list_view.read(cx).bounds().size;
        if previous_size.width > px(1.0)
            && previous_size.height > px(1.0)
            && (previous_size.width - bounds.size.width).abs() > px(0.5)
            && self.initial_frame_ready_delivered
            && self.viewport_resize_anchor.is_none()
        {
            self.viewport_resize_anchor = self.capture_viewport_resize_anchor(
                f32::from(bounds.size.width - previous_size.width),
                f32::from(bounds.size.height - previous_size.height),
                cx,
            );
        }
        self.bounds = bounds;
        self.display_scale = window.scale_factor();
    }

    pub(super) fn visible_ids(&self, cx: &App) -> Vec<String> {
        self.list_view.read(cx).visible_ids().to_vec()
    }

    pub fn did_move_to_window(&mut self, is_in_window: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.is_in_window = is_in_window;
        self.list_view
            .update(cx, |list_view, cx| list_view.did_move_to_window(is_in_window, window, cx));
        self.update_meaningful_activity(cx);
        self.refresh_speculative_preparation(window, cx);
        if !is_in_window {
            self.cancel_directional_media_prefetch(cx);
            self.cancel_remote_page_prefetch(window, cx);
        } else if self.initial_frame_ready_delivered {
            let visible_ids = self.visible_ids(cx);
            self.prefetch_directional_media(&visible_ids, cx);
        }
        if is_in_window && self.pending_initial_frame_reveal.is_some() {
            cx.defer_in(window, |this, window, cx| {
                this.deliver_pending_initial_frame_reveal_if_possible(window, cx);
            });
        }
        if !self.tracks_system_keyboard {
            return;
        }
        if !is_in_window {
            self.keyboard_target_overlap = 0.0;
            self.finish_configured_keyboard_transition(window, cx);
            return;
        }
        cx.defer_in(window, |this, window, cx| {
            if !this.is_in_window
                || !this.tracks_system_keyboard
                || this.list_view.read(cx).has_active_keyboard_viewport_transition()
            {
                return;
            }
            this.keyboard_target_overlap = this.current_keyboard_guide_overlap();
            this.finish_configured_keyboard_transition(window, cx);
        });
    }

    pub(super) fn application_did_become_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.initial_frame_ready_delivered {
            return;
        }
        self.last_reported_mark_candidate_ids.clear();
        let visible_ids = self.visible_ids(cx);
        self.report_visible_mark_candidates(&visible_ids, window, cx);
        self.prefetch_directional_media(&visible_ids, cx);
    }

    pub(super) fn application_did_enter_background(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_directional_media_prefetch(cx);
        self.cancel_remote_page_prefetch(window, cx);
        self.persist_scroll_position(None, cx);
        ChatScrollPositionStore::flush_pending_to_disk(cx);
        self.keyboard_target_is_visible = false;
        if !self.tracks_system_keyboard {
            return;
        }
        self.keyboard_target_overlap = 0.0;
        self.finish_configured_keyboard_transition(window, cx);
    }
}

impl Render for RiseMessageManualListView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity();
        div()
            .relative()
            .size_full()
            .child(
                canvas(
                    move |bounds, window, cx| {
                        this.update(cx, |this, cx| this.layout_subviews(bounds, window, cx));
                        Self::perform_pending_fallback_measurement(&this, window, cx);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(
                div()
                    .size_full()
                    .opacity(self.list_alpha)
                    .child(AnyView::from(self.list_view.clone()).cached(StyleRefinement::default().size_full())),
            )
    }
}

pub(super) fn rise_message_rendered_environment_differs(
    previous: &RiseMessageCollectionConfiguration,
    next: &RiseMessageCollectionConfiguration,
) -> bool {
    previous.layout_direction != next.layout_direction
        || previous.color_scheme != next.color_scheme
        || previous.locale.identifier() != next.locale.identifier()
        || previous.calendar.identifier != next.calendar.identifier
        || previous.time_zone.identifier() != next.time_zone.identifier()
        || previous.text_delta.to_bits() != next.text_delta.to_bits()
}

/// what the completion of one committed transaction needs, captured at the commit
pub(super) struct RiseMessageManualListViewCommittedTransaction {
    pub(super) generation: u64,
    pub(super) new_configuration: Rc<RiseMessageCollectionConfiguration>,
    pub(super) committed_candidate: RiseMessageManualListViewPreparedSnapshotCandidate,
    pub(super) next_native_ids: HashSet<String>,
    pub(super) items_by_id: Rc<HashMap<String, RiseManualListItem>>,
    pub(super) transaction_scroll_request_id: Option<Uuid>,
    pub(super) reset_requested: bool,
    pub(super) parks_on_scroll_target: bool,
    pub(super) did_prepend: bool,
    pub(super) did_append: bool,
    pub(super) travels_to_own_message: bool,
}

impl RiseMessageManualListView {
    pub(super) fn is_user_scrolling(&self, cx: &App) -> bool {
        let metrics = self.list_view.read(cx).scroll_metrics();
        let menu_closed_moments_ago = rise_ui::shared::try_read::<HoldContextMenuPresenter>(cx)
            .is_some_and(|presenter| presenter.closed_moments_ago());
        let tracking = metrics.is_tracking && !menu_closed_moments_ago;
        tracking || metrics.is_dragging || metrics.is_decelerating
    }
}

pub(super) fn rise_message_row_vertical_paddings(group: ChatMessageRenderEntryGroupFlags) -> (f32, f32) {
    (
        if group.is_first {
            MessageSizes::list_first_in_group_top_padding()
        } else {
            MessageSizes::list_continuation_top_padding()
        },
        if group.is_last { MessageSizes::list_last_in_group_bottom_padding() } else { 0.0 },
    )
}

impl RiseMessageManualListView {
    /// an id the item table cant answer is a bookkeeping mismatch, not a reason to kill the app in
    /// front of the reader
    /// a one point spacer keeps the transaction counts intact
    pub(super) fn missing_item_placeholder(id: &str, stage: &str, _window: &mut Window, _cx: &mut App) -> RiseManualListItem {
        debug_assert!(false, "Missing {stage} manual-list item for {id}");
        RiseManualListItem::new(
            1.0,
            "RiseMessageMissingItemPlaceholder".to_owned(),
            0,
            RiseManualListItemMountPolicy::Immediate,
            None,
            None,
            None,
            None,
            Rc::new(|reusable_view: Option<AnyView>, _window: &mut Window, cx: &mut App| {
                reusable_view.unwrap_or_else(|| cx.new(|_| RiseManualListViewPlaceholderView).into())
            }),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct RiseMessageManualListViewHistoryEdgeProximity {
    pub(super) distance: f32,
    pub(super) load_threshold: f32,
    pub(super) rearm_threshold: f32,
    pub(super) toward_velocity: f32,
}

pub(super) fn rise_message_append_prefetch_url(
    urls: &mut Vec<String>,
    seen: &mut HashSet<String>,
    limit: usize,
    raw: Option<&str>,
) {
    if urls.len() >= limit {
        return;
    }
    let Some(url) = RemoteMediaURL::optional_normalized(raw) else {
        return;
    };
    if seen.insert(url.clone()) {
        urls.push(url);
    }
}

impl RiseMessageManualListView {
    pub(super) fn playing_video_note_message_id(cx: &mut App) -> Option<String> {
        let store = VideoNotePlayerStore::shared(cx);
        let store = store.read(cx);
        (store.is_active() && !store.message_id().is_empty()).then(|| store.message_id().to_owned())
    }

    /// default update animation with empty content ids
    pub(super) fn row_slide_animation() -> RiseManualListTransactionUpdateAnimation {
        RiseManualListTransactionUpdateAnimation::new(0.34, 0.86, HashSet::new(), HashSet::new(), HashSet::new(), HashSet::new())
    }

    pub(super) fn enqueue_rendered_items_animated(
        &mut self,
        configuration: &Rc<RiseMessageCollectionConfiguration>,
        anchor: RiseManualListTransactionAnchor,
        allows_application_during_scroll: bool,
        animation: Option<RiseManualListTransactionUpdateAnimation>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.applied_source_version != Some(configuration.snapshot.source_version)
            || self.rendered_configuration_revision != self.configuration_revision
            || (self.rendered_preparation_width - f32::from(self.bounds.size.width)).abs() > 0.5
        {
            self.pending_width_repreparation = true;
            return;
        }
        let batch = RiseMessageManualListViewPreparedNativeBatch {
            chat_id: configuration.chat_id.clone(),
            source_version: configuration.snapshot.source_version,
            configuration_revision: self.configuration_revision,
            width: self.rendered_preparation_width,
            layouts: self.native_layouts.clone(),
            versions: self.native_layout_versions.clone(),
            ownership_by_id: self.native_ownership_by_id.clone(),
            entry_range: 0..configuration.snapshot.entries.len(),
            purpose: RiseMessageManualListViewNativePreparationPurpose::Foreground,
            hosted_text: self.hosted_text.clone(),
        };
        let candidate = RiseMessageManualListViewPreparedSnapshotCandidate {
            configuration: configuration.clone(),
            configuration_revision: self.configuration_revision,
            native_batch: batch,
            fallback_heights: self.measured_heights.clone(),
            history_edge_load_generation: None,
        };
        let native_ids = self.rendered_native_ids.clone();
        let item_table = self.make_manual_items(&candidate, &native_ids, &HashSet::new(), window, cx);
        let ids = configuration.snapshot.ids.clone();
        let preparation_width = self.rendered_preparation_width;
        let empty_ids: Vec<String> = Vec::new();
        let edge_mutation = Self::edge_mutation(
            self.rendered_configuration.as_ref().map_or(&empty_ids, |rendered| &rendered.snapshot.ids),
            &ids,
        );

        self.apply_generation = self.apply_generation.wrapping_add(1);
        let generation = self.apply_generation;
        self.is_applying_snapshot = true;
        let provider_items = item_table.items_by_id.clone();
        let completion_items = item_table.items_by_id;
        let completion_configuration = configuration.clone();
        let weak = cx.weak_entity();
        let completion_weak = weak.clone();
        let discard_weak = weak;
        self.list_view.update(cx, |list_view, cx| {
            list_view.enqueue(
                RiseManualListTransaction::new(
                    ids,
                    anchor,
                    Some(preparation_width),
                    Some(item_table.updated_ids),
                    animation,
                    None,
                    edge_mutation,
                    allows_application_during_scroll,
                    HashSet::new(),
                    Rc::new(move |id: &str, _width: f32, window: &mut Window, cx: &mut App| {
                        provider_items
                            .get(id)
                            .cloned()
                            .unwrap_or_else(|| RiseMessageManualListView::missing_item_placeholder(id, "refreshed", window, cx))
                    }),
                ),
                Some(Box::new(move |window: &mut Window, cx: &mut App| {
                    window.defer(cx, move |window, cx| {
                        let _ = completion_weak.update(cx, |this, cx| {
                            if this.apply_generation != generation {
                                return;
                            }
                            this.manual_items_by_id = completion_items;
                            this.manual_items_width = preparation_width;
                            this.rendered_configuration = Some(completion_configuration);
                            this.is_applying_snapshot = false;
                            this.report_visible_mark_candidates_after_commit(window, cx);
                            this.flush_deferred_work_if_possible(window, cx);
                            this.schedule_initial_frame_readiness_if_needed(window, cx);
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

    pub(super) fn edge_mutation(old_ids: &[String], new_ids: &[String]) -> Option<RiseManualListTransactionEdgeMutation> {
        if old_ids == new_ids {
            return Some(RiseManualListTransactionEdgeMutation::default());
        }

        let mut old_indices: HashMap<&str, usize> = HashMap::with_capacity(old_ids.len());
        for (index, id) in old_ids.iter().enumerate() {
            old_indices.entry(id.as_str()).or_insert(index);
        }
        let mut best_old_start = 0;
        let mut best_new_start = 0;
        let mut best_length = 0;
        let mut run_old_start = 0;
        let mut run_new_start = 0;
        let mut run_length = 0;
        let mut previous_old_index: Option<usize> = None;

        for (new_index, id) in new_ids.iter().enumerate() {
            let Some(old_index) = old_indices.get(id.as_str()).copied() else {
                run_length = 0;
                previous_old_index = None;
                continue;
            };
            if previous_old_index.is_some_and(|previous| old_index == previous + 1) {
                run_length += 1;
            } else {
                run_old_start = old_index;
                run_new_start = new_index;
                run_length = 1;
            }
            previous_old_index = Some(old_index);
            if run_length > best_length {
                best_old_start = run_old_start;
                best_new_start = run_new_start;
                best_length = run_length;
            }
        }

        if old_ids[best_old_start..best_old_start + best_length] != new_ids[best_new_start..best_new_start + best_length] {
            return None;
        }
        Some(RiseManualListTransactionEdgeMutation::new(
            best_old_start,
            old_ids.len() - best_old_start - best_length,
            best_new_start,
            new_ids.len() - best_new_start - best_length,
        ))
    }

    pub(super) fn stop_after_window_closed(&mut self, cx: &mut Context<Self>) {
        self.persist_scroll_position(None, cx);
        ChatScrollPositionStore::flush_pending_to_disk(cx);
        self.cancel_directional_media_prefetch(cx);
    }

    pub(super) fn window_counts_as_active(window: &Window) -> bool {
        window.is_window_active() || crate::app::dev_drive::ENABLED
    }
}

/// hosted row waiting for the next frame, gpui has no layout pass outside one
pub(super) struct RiseMessageManualListViewFallbackMeasurementRequest {
    pub(super) view: Entity<RiseManualMessageHostingView>,
    pub(super) width: f32,
    pub(super) configuration: Rc<RiseMessageCollectionConfiguration>,
    pub(super) rows: Vec<(Arc<ChatMessageRenderEntry>, Option<FormattedTextInlinePreparation>)>,
    pub(super) budget_seconds: f64,
    pub(super) sender: oneshot::Sender<Vec<f32>>,
}

pub(super) struct RiseMessageManualListViewFallbackHydration {
    pub(super) requested_range: Range<usize>,
    pub(super) native_count: i64,
    pub(super) exact_count: i64,
    pub(super) cache_hit_count: i64,
    pub(super) unresolved_indices: Vec<usize>,
    pub(super) critical_unresolved: Vec<usize>,
    pub(super) ordered_unresolved: Vec<usize>,
}

pub(super) struct RiseMessageManualListViewWorkerCancellation(pub(super) Arc<AtomicBool>);

impl Drop for RiseMessageManualListViewWorkerCancellation {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

impl RiseMessageManualListView {
    pub(super) fn perform_pending_fallback_measurement(this: &Entity<Self>, window: &mut Window, cx: &mut App) {
        let Some(request) = this.update(cx, |this, _| this.pending_fallback_measurement.take()) else {
            return;
        };
        let width = request.width;
        let started_at = rise_core::foundation::foundation_date::RiseClock::monotonic();
        let mut heights = Vec::with_capacity(request.rows.len());
        for (entry, prepared_text) in request.rows {
            let configuration = &request.configuration;
            let height = request.view.update(cx, |view, cx| {
                view.configure(&entry, configuration, prepared_text, Some(width), true, None, None, cx);
                view.measured_height(width, window, cx)
            });
            heights.push(height);
            let elapsed = rise_core::foundation::foundation_date::RiseClock::monotonic().saturating_sub(started_at);
            if elapsed.as_secs_f64() >= request.budget_seconds {
                break;
            }
        }
        let _ = request.sender.send(heights);
    }

    pub(super) fn is_at_exact_bottom(&self, cx: &App) -> bool {
        if self
            .rendered_configuration
            .as_ref()
            .or(self.configuration.as_ref())
            .is_some_and(|configuration| configuration.can_load_newer)
        {
            return false;
        }
        self.distance_from_bottom(cx) <= RiseMessageScrollPositionPersistencePolicy::EXACT_BOTTOM_TOLERANCE
    }

    pub(super) fn distance_from_bottom(&self, cx: &App) -> f32 {
        f32::from(self.list_view.read(cx).scroll_metrics().distance_from_bottom)
    }
}
