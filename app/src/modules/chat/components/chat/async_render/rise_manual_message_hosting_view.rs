use std::cell::Cell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, AnyView, App, AppContext, AvailableSpace, Bounds, Context, ElementId, Entity, EntityId, Global, Hsla,
    DispatchPhase, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ParentElement, Pixels, Point, Render, ScrollDelta, ScrollWheelEvent, Size, Styled, Task, TouchPhase, WeakEntity,
    Window, canvas, div, point, px, transparent_black,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::rise_app_events::RiseAppEvents;
use rise_platform::haptics::{ImpactStyle, impact};
use rise_ui::animated_media_playback_scope::{
    ANIMATED_MEDIA_VIEWPORT_SCOPE_DID_CHANGE, AnimatedMediaViewportScopeProviding, AnimatedMediaViewportScopeViewKey,
};
use rise_ui::hold_context_menu_ui::bubble_scroll_passthrough::BubbleScrollPassthrough;
use rise_ui::swiftui::text_selection::{
    TextDragAxis, gesture_plate_took_press, mark_text_press_swiped, text_drag_axis, text_glyph_took_press,
};
use rise_ui::icon_ui::icon_ui::IconUi;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::environment::env_scope;
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::prelude::measure;
use rise_ui::view_anchor_probe::{ViewAnchorProbe, ViewAnchorProbeReader};
use rise_ui::view_screen_snapshot::{HoldContextMenuSnapshotRenderRoot, ViewScreenSource};

use crate::core::ui::formatted_text_ui::FormattedTextInlinePreparation;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::core::ui::list_engine::rise_manual_list_integration::{
    RiseManualListConformances, RiseManualListPreparedHeightValidating, RiseManualListViewportVisibilityReceiving,
};
use crate::modules::chat::components::chat::async_render::chat_message_render_snapshot::{
    ChatMessageRenderEntry, ChatMessageRenderEntryGroupFlags, ChatMessageRenderEntryKind,
};
use crate::modules::chat::components::chat::async_render::chat_performance_trace::ChatPerformanceTrace;
use crate::modules::chat::components::chat::async_render::list_engine::rise_list_jump_debug::RiseListJumpDebug;
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_types::{
    RiseManualListReusableView, RiseManualListSelectionPresentable,
};
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_view::RiseManualListView;
use crate::modules::chat::components::chat::async_render::list_engine::rise_vertical_scroll_gesture_gate::{
    RiseVerticalScrollGestureGate, cancel_enclosing_scroll_pan_for_current_touch,
};
use crate::modules::chat::components::chat::async_render::rise_message_layout_measurement::RiseMessageLayoutMeasurementKey;
use crate::modules::chat::components::chat::async_render::rise_message_list_view::{
    RiseMessageCollectionCellRoot, RiseMessageCollectionConfiguration,
};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuSourceLift;
use crate::modules::chat::components::chat::bubble::message_content::ChatMessageViewportWidthKey;
use crate::modules::chat::components::chat::message_list::MessageList;
use crate::modules::chat::shared::chat_search_highlight::{ChatMessageSearchHighlight, ChatSearchHighlightWordsKey};
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::chat::shared::message_permissions::{ChatMessageAction, MessagePermissions};
use crate::modules::chat::stores::message::message_actions::message_types::{GetMessageMessage, MessageContentType};

pub enum RiseHostedRowAuditPolicy {}

impl RiseHostedRowAuditPolicy {
    pub const INITIAL_DELAY: f64 = 0.12;
    pub const VERIFICATION_DELAY: f64 = 0.08;
    pub const SCROLLING_RETRY_DELAY: f64 = 0.2;
    pub const OVERFLOW_DELAY: f64 = 1.0 / 60.0;
    pub const MAX_AUDITS_PER_TURN: usize = 3;
    pub const MAX_VISIBLE_AUDITS_PER_SCROLLING_TURN: usize = 1;
}

pub enum RiseHostedRowViewportAuditPolicy {}

impl RiseHostedRowViewportAuditPolicy {
    pub const MAXIMUM_MEASUREMENTS_PER_PASS: usize = usize::MAX;

    pub fn prioritized_indices(range: Range<usize>) -> Vec<usize> {
        if range.is_empty() {
            return Vec::new();
        }
        let center = range.start + range.len() / 2;
        let mut indices: Vec<usize> = range.collect();
        indices.sort_by_key(|index| (index.abs_diff(center), *index));
        indices
    }
}

pub enum RiseHostedRowHeightAuditLifecyclePolicy {}

impl RiseHostedRowHeightAuditLifecyclePolicy {
    pub fn should_schedule_after_attachment(is_attached_to_window: bool, needs_validation: bool) -> bool {
        is_attached_to_window && needs_validation
    }
}

struct RiseHostedRowHeightAuditCoordinatorPendingAudit {
    view: WeakEntity<RiseManualMessageHostingView>,
    generation: u64,
    pass: u32,
    deadline: f64,
    token: u64,
}

#[derive(Default)]
struct RiseHostedRowHeightAuditCoordinator {
    pending: HashMap<EntityId, RiseHostedRowHeightAuditCoordinatorPendingAudit>,
    scheduled_work_item: Option<Task<()>>,
    scheduled_deadline: Option<f64>,
    next_token: u64,
}

impl Global for RiseHostedRowHeightAuditCoordinator {}

impl RiseHostedRowHeightAuditCoordinator {
    fn now() -> f64 {
        RiseClock::monotonic().as_secs_f64()
    }

    fn schedule(
        view: WeakEntity<RiseManualMessageHostingView>,
        key: EntityId,
        generation: u64,
        delay: f64,
        cx: &mut App,
    ) {
        let coordinator = cx.default_global::<Self>();
        coordinator.next_token = coordinator.next_token.wrapping_add(1);
        let token = coordinator.next_token;
        coordinator.pending.insert(
            key,
            RiseHostedRowHeightAuditCoordinatorPendingAudit {
                view,
                generation,
                pass: 0,
                deadline: Self::now() + delay,
                token,
            },
        );
        Self::arm_next_audit_if_needed(cx);
    }

    fn cancel(key: EntityId, cx: &mut App) {
        let coordinator = cx.default_global::<Self>();
        coordinator.pending.remove(&key);
        if !coordinator.pending.is_empty() {
            return;
        }
        coordinator.scheduled_work_item = None;
        coordinator.scheduled_deadline = None;
    }

    fn arm_next_audit_if_needed(cx: &mut App) {
        let coordinator = cx.default_global::<Self>();
        let earliest = coordinator.pending.values().map(|entry| entry.deadline).reduce(f64::min);
        let Some(earliest) = earliest else {
            coordinator.scheduled_work_item = None;
            coordinator.scheduled_deadline = None;
            return;
        };
        if coordinator.scheduled_deadline.is_some_and(|deadline| deadline <= earliest + 0.000_5) {
            return;
        }
        let delay = (earliest - Self::now()).max(0.0);
        coordinator.scheduled_deadline = Some(earliest);
        let work_item = cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_secs_f64(delay)).await;
            cx.update(|cx| Self::run_due_audits(cx));
        });
        cx.default_global::<Self>().scheduled_work_item = Some(work_item);
    }

    fn run_due_audits(cx: &mut App) {
        let pending_count = cx.default_global::<Self>().pending.len();
        let trace_token = ChatPerformanceTrace::shared()
            .begin_operation("height.audit_batch", HashMap::from([("pending".to_owned(), pending_count.to_string())]));

        let now = Self::now();
        let due_keys: Vec<EntityId> = {
            let coordinator = cx.default_global::<Self>();
            coordinator.scheduled_deadline = None;
            coordinator
                .pending
                .iter()
                .filter(|(_, entry)| entry.deadline <= now + 0.001)
                .map(|(key, _)| *key)
                .collect()
        };
        let mut moving_by_scroll_view: HashMap<EntityId, bool> = HashMap::new();
        let mut completed_this_turn = 0usize;

        for key in due_keys {
            let Some((view, generation, pass, token)) = cx
                .default_global::<Self>()
                .pending
                .get(&key)
                .map(|entry| (entry.view.clone(), entry.generation, entry.pass, entry.token))
            else {
                continue;
            };
            let Some(view) =
                view.upgrade().filter(|view| view.read(cx).is_height_audit_eligible(generation))
            else {
                cx.default_global::<Self>().pending.remove(&key);
                continue;
            };

            let is_moving = match view.read(cx).height_audit_scroll_view() {
                Some(scroll_view) => {
                    let scroll_key = scroll_view.entity_id();
                    *moving_by_scroll_view.entry(scroll_key).or_insert_with(|| {
                        let metrics = scroll_view.read(cx).scroll_metrics();
                        metrics.is_tracking || metrics.is_dragging || metrics.is_decelerating
                    })
                }
                None => false,
            };

            if is_moving && !view.read(cx).is_visible_for_height_audit(cx) {
                Self::postpone(key, now + RiseHostedRowAuditPolicy::SCROLLING_RETRY_DELAY, cx);
                continue;
            }
            let audit_limit = if is_moving {
                RiseHostedRowAuditPolicy::MAX_VISIBLE_AUDITS_PER_SCROLLING_TURN
            } else {
                RiseHostedRowAuditPolicy::MAX_AUDITS_PER_TURN
            };
            if completed_this_turn >= audit_limit {
                let delay = if is_moving {
                    RiseHostedRowAuditPolicy::SCROLLING_RETRY_DELAY
                } else {
                    RiseHostedRowAuditPolicy::OVERFLOW_DELAY
                };
                Self::postpone(key, now + delay, cx);
                continue;
            }

            let completed_validation = RiseManualMessageHostingView::perform_height_audit(&view, cx);
            completed_this_turn += 1;

            let coordinator = cx.default_global::<Self>();
            if coordinator.pending.get(&key).is_none_or(|entry| entry.token != token) {
                continue;
            }
            if !completed_validation && pass == 0 {
                if let Some(entry) = coordinator.pending.get_mut(&key) {
                    entry.pass = 1;
                    entry.deadline = now + RiseHostedRowAuditPolicy::VERIFICATION_DELAY;
                }
            } else {
                coordinator.pending.remove(&key);
            }
        }

        let remaining = cx.default_global::<Self>().pending.len();
        ChatPerformanceTrace::shared().end_operation(
            trace_token,
            1.0,
            HashMap::from([
                ("completed".to_owned(), completed_this_turn.to_string()),
                ("remaining".to_owned(), remaining.to_string()),
            ]),
        );
        Self::arm_next_audit_if_needed(cx);
    }

    fn postpone(key: EntityId, deadline: f64, cx: &mut App) {
        if let Some(entry) = cx.default_global::<Self>().pending.get_mut(&key) {
            entry.deadline = deadline;
        }
    }
}

/// height mismatch callback of configure, gets the measured natural height of the row
pub type RiseManualMessageHostingViewOnHeightMismatch = Rc<dyn Fn(f32, &mut App)>;

type RiseManualMessageHostingRoot = Rc<dyn Fn() -> AnyElement>;

/// is animated media visible in viewport, as cells read it through the environment
#[derive(Default)]
struct RiseManualMessageHostingViewportScope {
    is_visible: Cell<bool>,
}

impl AnimatedMediaViewportScopeProviding for RiseManualMessageHostingViewportScope {
    fn is_animated_media_visible_in_viewport(&self) -> bool {
        self.is_visible.get()
    }
}

/// the one content entity of the row, kept across configure and reuse
struct RiseManualMessageHostingContent {
    root: Option<RiseManualMessageHostingRoot>,
    viewport_scope: Rc<RiseManualMessageHostingViewportScope>,
}

impl Render for RiseManualMessageHostingContent {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let Some(root) = self.root.clone() else {
            return div().into_any_element();
        };
        let scope: Rc<dyn AnimatedMediaViewportScopeProviding> = self.viewport_scope.clone();
        env_scope::<AnimatedMediaViewportScopeViewKey>(Some(scope)).child(root()).into_any_element()
    }
}

/// states of the reply swipe recognizer the row has to tell apart
#[derive(Clone, Copy, PartialEq)]
enum RiseManualMessageReplyPanPhase {
    Possible,
    Began,
    Failed,
}

/// row of the manual message list that hosts the bubble
/// it never lays out on the configure path, configure runs before the engine assigns the frame
struct RiseManualMessageHostingConfigureKey {
    configuration: Rc<RiseMessageCollectionConfiguration>,
    entry_id: String,
    kind: ChatMessageRenderEntryKind,
    content_version: i64,
    message: Option<Arc<GetMessageMessage>>,
    group: Option<ChatMessageRenderEntryGroupFlags>,
    highlight_words: Vec<String>,
    viewport_width: f32,
    measurement_only: bool,
}

impl RiseManualMessageHostingConfigureKey {
    fn same(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.configuration, &other.configuration)
            && self.entry_id == other.entry_id
            && self.kind == other.kind
            && self.content_version == other.content_version
            && match (&self.message, &other.message) {
                (Some(lhs), Some(rhs)) => Arc::ptr_eq(lhs, rhs),
                (None, None) => true,
                _ => false,
            }
            && self.group == other.group
            && self.highlight_words == other.highlight_words
            && self.viewport_width == other.viewport_width
            && self.measurement_only == other.measurement_only
    }
}

pub struct RiseManualMessageHostingView {
    hosted_view: Option<Entity<RiseManualMessageHostingContent>>,
    configure_key: Option<RiseManualMessageHostingConfigureKey>,
    hosted_view_probe: ViewAnchorProbe,
    viewport_scope: Rc<RiseManualMessageHostingViewportScope>,
    reply_hint_tint_color: Hsla,
    reply_hint_alpha: Animated<f32>,
    reply_hint_scale: Animated<f32>,
    hosted_translation_x: Animated<f32>,
    reply_pan_id: ElementId,
    reply_pan_phase: Option<RiseManualMessageReplyPanPhase>,
    reply_pan_translation: Point<Pixels>,
    vertical_scroll_gate: RiseVerticalScrollGestureGate,
    height_audit_generation: u64,
    validated_height_audit_generation: Option<u64>,
    height_audit_scroll_view: Option<WeakEntity<RiseManualListView>>,
    expected_height: Option<f32>,
    last_reported_natural_height: f32,
    on_height_mismatch: Option<RiseManualMessageHostingViewOnHeightMismatch>,
    pinned_content_height: Option<f32>,
    pinned_content_width: Option<f32>,
    pinned_row_height: Option<f32>,
    pin_follows_row_height: bool,
    debug_is_own_bubble: bool,
    allows_reply: bool,
    reply_armed: bool,
    reply_pulls_right: bool,
    mouse_press: Option<Point<Pixels>>,
    mouse_axis: Option<TextDragAxis>,
    on_reply: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
    on_row_double_tap: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
    row_double_tap_armed: bool,
    selection_translation_x: f32,
    bounds: Option<Bounds<Pixels>>,
    window_scale: Option<f32>,
    natural_content_height: Option<f32>,
    did_report_attachment: Cell<bool>,
    entity_id: EntityId,
}

impl RiseManualMessageHostingView {
    pub const REUSE_IDENTIFIER: &'static str = "RiseManualMessageHostingView";

    const UNMEASURED_HEIGHT: f32 = 64.0;
    const REPLY_HINT_SIDE: f32 = 18.0;
    const REPLY_MAXIMUM_PULL: f32 = 80.0;
    const REPLY_ARM_PULL: f32 = 56.0;

    pub fn reuse_identifier(entry: &ChatMessageRenderEntry) -> String {
        match entry.kind {
            ChatMessageRenderEntryKind::DateSeparator => format!("{}.date", Self::REUSE_IDENTIFIER),
            ChatMessageRenderEntryKind::TopicSeparator => format!("{}.topic", Self::REUSE_IDENTIFIER),
            ChatMessageRenderEntryKind::UnreadSeparator => format!("{}.unread", Self::REUSE_IDENTIFIER),
            ChatMessageRenderEntryKind::System => format!("{}.system", Self::REUSE_IDENTIFIER),
            ChatMessageRenderEntryKind::Message => format!(
                "{}.message.{}",
                Self::REUSE_IDENTIFIER,
                entry.message.as_ref().map_or("unknown", |message| message.content_type.raw_value())
            ),
        }
    }

    pub fn new(cx: &mut Context<Self>) -> Self {
        RiseManualListConformances::register_reusable_view::<Self>(cx);
        RiseManualListConformances::register_selection_presentable::<Self>(cx);
        RiseManualListConformances::register_viewport_visibility_receiving::<Self>(cx);
        RiseManualListConformances::register_prepared_height_validating::<Self>(cx);
        RiseManualListConformances::register_moved_replay_row::<Self>(cx);
        let reply_pan_id = ElementId::Name("rise-manual-message-reply-pan".into());
        let mut vertical_scroll_gate = RiseVerticalScrollGestureGate::new(cx.entity_id());
        vertical_scroll_gate.require_vertical_intent_to_fail(reply_pan_id.clone());
        Self {
            hosted_view: None,
            configure_key: None,
            hosted_view_probe: ViewAnchorProbe::new(),
            viewport_scope: Rc::new(RiseManualMessageHostingViewportScope::default()),
            reply_hint_tint_color: transparent_black(),
            reply_hint_alpha: Animated::new(0.0),
            reply_hint_scale: Animated::new(1.0),
            hosted_translation_x: Animated::new(0.0),
            reply_pan_id,
            reply_pan_phase: None,
            reply_pan_translation: Point::default(),
            vertical_scroll_gate,
            height_audit_generation: 0,
            validated_height_audit_generation: None,
            height_audit_scroll_view: None,
            expected_height: None,
            last_reported_natural_height: 0.0,
            on_height_mismatch: None,
            pinned_content_height: None,
            pinned_content_width: None,
            pinned_row_height: None,
            pin_follows_row_height: false,
            debug_is_own_bubble: false,
            allows_reply: false,
            reply_armed: false,
            reply_pulls_right: false,
            mouse_press: None,
            mouse_axis: None,
            on_reply: None,
            on_row_double_tap: None,
            row_double_tap_armed: false,
            selection_translation_x: 0.0,
            bounds: None,
            window_scale: None,
            natural_content_height: None,
            did_report_attachment: Cell::new(false),
            entity_id: cx.entity_id(),
        }
    }

    pub fn set_height_audit_scroll_view(&mut self, scroll_view: WeakEntity<RiseManualListView>) {
        self.height_audit_scroll_view = Some(scroll_view);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn configure(
        &mut self,
        entry: &ChatMessageRenderEntry,
        configuration: &Rc<RiseMessageCollectionConfiguration>,
        prepared_text: Option<FormattedTextInlinePreparation>,
        viewport_width: Option<f32>,
        measurement_only: bool,
        expected_height: Option<f32>,
        on_height_mismatch: Option<RiseManualMessageHostingViewOnHeightMismatch>,
        cx: &mut Context<Self>,
    ) {
        let kind_field = entry.message.as_ref().map_or("system", |message| message.content_type.raw_value()).to_owned();
        let trace_token = ChatPerformanceTrace::shared().begin_operation(
            "hosting.configure",
            HashMap::from([
                ("kind".to_owned(), kind_field),
                ("measurement_only".to_owned(), if measurement_only { "1" } else { "0" }.to_owned()),
                ("has_expected_height".to_owned(), if expected_height.is_none() { "0" } else { "1" }.to_owned()),
            ]),
        );

        let pin_before = (
            self.expected_height,
            self.pinned_content_height,
            self.pinned_content_width,
            self.pinned_row_height,
            self.pin_follows_row_height,
        );
        self.height_audit_generation = self.height_audit_generation.wrapping_add(1);
        let audit_generation = self.height_audit_generation;
        RiseHostedRowHeightAuditCoordinator::cancel(cx.entity_id(), cx);
        self.validated_height_audit_generation = None;
        self.expected_height = expected_height;
        self.last_reported_natural_height = expected_height.unwrap_or(0.0);
        let is_audited = on_height_mismatch.is_some();
        self.on_height_mismatch = on_height_mismatch;
        self.pinned_content_height = None;
        self.pinned_content_width = None;
        self.pinned_row_height = expected_height;
        self.pin_follows_row_height = false;
        if RiseListJumpDebug::is_enabled() {
            self.debug_is_own_bubble = entry
                .message
                .as_ref()
                .is_some_and(|message| MessageList::is_own_bubble(message, &configuration.chat, cx));
        }
        self.configure_reply(entry, configuration, measurement_only, cx);
        self.configure_row_double_tap(entry, configuration, measurement_only, cx);

        let resolved_viewport_width = viewport_width
            .or_else(|| self.bounds.map(|bounds| f32::from(bounds.size.width)).filter(|width| *width > 0.0))
            .unwrap_or(1.0)
            .max(1.0);

        let highlight_words = match (&entry.search_highlight_ranges, &entry.message) {
            (Some(ranges), message) => ChatMessageSearchHighlight::words(
                message.as_ref().map_or("", |message| message.text.as_str()),
                ranges,
            ),
            (None, _) => Vec::new(),
        };
        let key = (prepared_text.is_none() && !is_audited).then(|| RiseManualMessageHostingConfigureKey {
            configuration: configuration.clone(),
            entry_id: entry.id.clone(),
            kind: entry.kind,
            content_version: entry.content_version,
            message: entry.message.clone(),
            group: entry.group,
            highlight_words: highlight_words.clone(),
            viewport_width: resolved_viewport_width,
            measurement_only,
        });
        let content_unchanged = self.hosted_view.is_some()
            && matches!((&key, &self.configure_key), (Some(next), Some(installed)) if next.same(installed));
        if !content_unchanged {
            let identity = RiseHostedRowIdentity { entry_id: entry.id.clone() };
            let entry = Rc::new(entry.clone());
            let configuration = configuration.clone();
            let root: RiseManualMessageHostingRoot = Rc::new(move || {
                let cell = RiseMessageCollectionCellRoot::new(entry.clone(), configuration.clone(), prepared_text.clone());
                env_scope::<RiseMessageLayoutMeasurementKey>(measurement_only)
                    .child(env_scope::<ChatMessageViewportWidthKey>(resolved_viewport_width).child(
                        env_scope::<ChatSearchHighlightWordsKey>(highlight_words.clone())
                            .child(identity.body(cell.into_any_element())),
                    ))
                    .into_any_element()
            });
            self.install(Some(root), cx);
            self.configure_key = key;
        }

        if !measurement_only
            && let Some(expected_height) = expected_height
            && !is_audited
            && self.hosted_view.is_some()
            && resolved_viewport_width > 1.0
        {
            self.pinned_content_height = Some(expected_height);
            self.pinned_content_width = Some(resolved_viewport_width);
            self.pin_follows_row_height = true;
        }
        if expected_height.is_some() && is_audited {
            RiseHostedRowHeightAuditCoordinator::schedule(
                cx.weak_entity(),
                cx.entity_id(),
                audit_generation,
                RiseHostedRowAuditPolicy::INITIAL_DELAY,
                cx,
            );
        }
        let pin_after = (
            self.expected_height,
            self.pinned_content_height,
            self.pinned_content_width,
            self.pinned_row_height,
            self.pin_follows_row_height,
        );
        if !content_unchanged || pin_after != pin_before {
            cx.notify();
        }
        ChatPerformanceTrace::shared().end_operation(trace_token, 1.0, HashMap::new());
    }

    /// one sizing pass of the measurement host
    /// runs only inside a frame, gpui has no layout pass outside one
    pub fn measured_height(&mut self, width: f32, window: &mut Window, cx: &mut App) -> f32 {
        let trace_token = ChatPerformanceTrace::shared().begin_operation("hosting.measure", HashMap::new());
        let height = match self.hosted_view.clone() {
            Some(hosted_view) => {
                self.bounds = Some(Bounds {
                    origin: point(px(0.0), px(0.0)),
                    size: gpui::size(px(width), px(Self::UNMEASURED_HEIGHT)),
                });
                Self::natural_height(&AnyView::from(hosted_view), width, window.scale_factor(), window, cx)
                    .unwrap_or(Self::UNMEASURED_HEIGHT)
            }
            None => Self::UNMEASURED_HEIGHT,
        };
        ChatPerformanceTrace::shared().end_operation(trace_token, 1.0, HashMap::new());
        height
    }

    /// needs prepared viewport height validation
    fn needs_validation(&self) -> bool {
        self.on_height_mismatch.is_some()
            && self.expected_height.is_some()
            && self.validated_height_audit_generation != Some(self.height_audit_generation)
    }

    fn configure_reply(
        &mut self,
        entry: &ChatMessageRenderEntry,
        configuration: &Rc<RiseMessageCollectionConfiguration>,
        measurement_only: bool,
        cx: &mut Context<Self>,
    ) {
        self.reset_reply_presentation(false, cx);
        self.reply_pan_phase = None;
        self.mouse_press = None;
        self.mouse_axis = None;
        let message = entry.message.as_ref().filter(|_| !measurement_only && entry.kind == ChatMessageRenderEntryKind::Message);
        let Some(message) = message else {
            self.allows_reply = false;
            self.on_reply = None;
            return;
        };
        let is_own = MessageList::is_own_bubble(message, &configuration.chat, cx);
        self.allows_reply = configuration.interactions_enabled
            && MessagePermissions::new(message, &configuration.chat, is_own).can_reply(cx);
        let message_id = message.id.clone();
        let reply_configuration = configuration.clone();
        self.on_reply = Some(Rc::new(move |window: &mut Window, cx: &mut App| {
            if let Some(on_action) = reply_configuration.on_action.as_ref() {
                on_action(&message_id, ChatMessageAction::REPLY, window, cx);
            }
        }));
        self.reply_hint_tint_color = configuration.theme_store.read(cx).current_theme().primary._100;
    }

    fn configure_row_double_tap(
        &mut self,
        entry: &ChatMessageRenderEntry,
        configuration: &Rc<RiseMessageCollectionConfiguration>,
        measurement_only: bool,
        cx: &mut Context<Self>,
    ) {
        self.row_double_tap_armed = false;
        let message = entry.message.as_ref().filter(|message| {
            !measurement_only
                && entry.kind == ChatMessageRenderEntryKind::Message
                && message.content_type != MessageContentType::Call
                && !configuration.chat.is_community_preview()
        });
        let Some(message) = message.filter(|message| configuration.can_trigger_quick_reaction(message, cx)) else {
            self.on_row_double_tap = None;
            return;
        };
        let message = Arc::clone(message);
        let reaction_configuration = configuration.clone();
        self.on_row_double_tap = Some(Rc::new(move |window: &mut Window, cx: &mut App| {
            reaction_configuration.trigger_quick_reaction(&message, window, cx);
        }));
    }

    fn handle_row_mouse_down(&mut self, event: &MouseDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let swipes = self.allows_reply
            && self.reply_pan_phase != Some(RiseManualMessageReplyPanPhase::Began)
            && !HoldContextMenuPresenter::has_active_request(cx)
            && !BubbleScrollPassthrough::with_shared(|passthrough| passthrough.contains(event.position));
        self.mouse_press = swipes.then_some(event.position);
        self.mouse_axis = None;
        self.row_double_tap_armed = event.click_count >= 2 && event.click_count % 2 == 0
            && self.on_row_double_tap.is_some()
            && !gesture_plate_took_press(event.position, cx)
            && !HoldContextMenuPresenter::has_active_request(cx)
            && !text_glyph_took_press(event.position, cx)
            && !BubbleScrollPassthrough::with_shared(|passthrough| passthrough.contains(event.position));
    }

    fn handle_row_mouse_up(&mut self, event: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        let armed = std::mem::take(&mut self.row_double_tap_armed);
        if !armed || event.click_count < 2 || event.click_count % 2 != 0 {
            return;
        }
        if let Some(on_row_double_tap) = self.on_row_double_tap.clone() {
            cx.defer_in(window, move |_, window, cx| on_row_double_tap(window, cx));
        }
    }

    fn handle_scroll_wheel(&mut self, event: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.vertical_scroll_gate.handle_scroll_wheel(event);
        if self.reply_pan_phase == Some(RiseManualMessageReplyPanPhase::Began) {
            return;
        }
        if event.touch_phase == TouchPhase::Started {
            self.reply_pan_translation = Point::default();
            self.reply_pan_phase = self.allows_reply.then_some(RiseManualMessageReplyPanPhase::Possible);
        }
        let Some(phase) = self.reply_pan_phase else {
            return;
        };
        match event.touch_phase {
            TouchPhase::Started | TouchPhase::Moved => {
                let ScrollDelta::Pixels(delta) = event.delta else {
                    return;
                };
                self.reply_pan_translation =
                    point(self.reply_pan_translation.x + delta.x, self.reply_pan_translation.y + delta.y);
                match phase {
                    RiseManualMessageReplyPanPhase::Possible => {
                        if f32::from(delta.x) == 0.0 && f32::from(delta.y) == 0.0 {
                            return;
                        }
                        if self.gesture_recognizer_should_begin(delta, event.position, cx) {
                            self.reply_pan_phase = Some(RiseManualMessageReplyPanPhase::Began);
                            self.reply_pulls_right = false;
                            self.mouse_press = None;
                            cancel_enclosing_scroll_pan_for_current_touch(cx);
                            self.reply_pan_did_change(cx);
                        } else {
                            self.reply_pan_phase = Some(RiseManualMessageReplyPanPhase::Failed);
                        }
                    }
                    RiseManualMessageReplyPanPhase::Began => self.reply_pan_did_change(cx),
                    RiseManualMessageReplyPanPhase::Failed => {}
                }
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                self.reply_pan_phase = None;
                if phase == RiseManualMessageReplyPanPhase::Began {
                    self.reply_pan_did_end(event.touch_phase == TouchPhase::Ended, window, cx);
                }
            }
        }
    }

    /// a begun pan follows the gesture wherever the pointer is, the list may slide under it
    fn handle_active_reply_scroll(&mut self, event: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.reply_pan_phase != Some(RiseManualMessageReplyPanPhase::Began) || self.mouse_press.is_some() {
            return;
        }
        match event.touch_phase {
            TouchPhase::Moved => {
                if let ScrollDelta::Pixels(delta) = event.delta {
                    self.reply_pan_translation =
                        point(self.reply_pan_translation.x + delta.x, self.reply_pan_translation.y + delta.y);
                    self.reply_pan_did_change(cx);
                }
            }
            TouchPhase::Ended | TouchPhase::Cancelled | TouchPhase::Started => {
                self.reply_pan_phase = None;
                self.reply_pan_did_end(event.touch_phase == TouchPhase::Ended, window, cx);
            }
        }
    }

    fn handle_reply_mouse_move(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(press) = self.mouse_press else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            self.handle_reply_mouse_release(false, window, cx);
            return;
        }
        if self.mouse_axis.is_none() {
            let Some(axis) = text_drag_axis(press, event.position) else {
                return;
            };
            let pulls_left = event.position.x < press.x;
            if axis != TextDragAxis::Swipe || HoldContextMenuPresenter::has_active_request(cx) {
                self.mouse_press = None;
                return;
            }
            mark_text_press_swiped(press, cx);
            if !pulls_left {
                self.mouse_press = None;
                return;
            }
            self.mouse_axis = Some(axis);
            self.reply_pan_phase = Some(RiseManualMessageReplyPanPhase::Began);
            self.reply_pulls_right = false;
        }
        self.reply_pan_translation = event.position - press;
        self.reply_pan_did_change(cx);
    }

    fn handle_reply_mouse_release(&mut self, completed: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(_) = self.mouse_press.take() else {
            return;
        };
        let began = self.mouse_axis.take().is_some();
        if began && self.reply_pan_phase == Some(RiseManualMessageReplyPanPhase::Began) {
            self.reply_pan_phase = None;
            self.reply_pan_did_end(completed, window, cx);
        }
    }

    fn gesture_recognizer_should_begin(&self, velocity: Point<Pixels>, start_location: Point<Pixels>, cx: &mut App) -> bool {
        if !self.allows_reply || HoldContextMenuPresenter::has_active_request(cx) {
            return false;
        }
        if !self.vertical_scroll_gate.allows_pan(&self.reply_pan_id) {
            return false;
        }
        if BubbleScrollPassthrough::with_shared(|passthrough| passthrough.contains(start_location)) {
            return false;
        }
        let velocity_x = f32::from(velocity.x);
        velocity_x < 0.0 && velocity_x.abs() > f32::from(velocity.y).abs()
    }

    fn reply_pan_did_change(&mut self, cx: &mut Context<Self>) {
        let towards = f32::from(self.reply_pan_translation.x);
        let towards = if self.reply_pulls_right { towards } else { -towards };
        let pull = towards.clamp(0.0, Self::REPLY_MAXIMUM_PULL);
        let damped = if pull <= Self::REPLY_ARM_PULL {
            pull
        } else {
            Self::REPLY_ARM_PULL + (pull - Self::REPLY_ARM_PULL) * 0.35
        };
        let progress = (pull / Self::REPLY_ARM_PULL).min(1.0);
        let shift = if self.reply_pulls_right { damped } else { -damped };
        self.hosted_translation_x.jump(self.selection_translation_x + shift);
        self.reply_hint_alpha.jump(progress);
        self.reply_hint_scale.jump(0.6 + progress * 0.4);
        let next_armed = pull >= Self::REPLY_ARM_PULL;
        if next_armed != self.reply_armed {
            self.reply_armed = next_armed;
            if next_armed {
                let _ = impact(ImpactStyle::Medium);
            }
        }
        cx.notify();
    }

    fn reply_pan_did_end(&mut self, did_finish: bool, window: &mut Window, cx: &mut Context<Self>) {
        let should_reply = self.reply_armed && did_finish;
        self.reset_reply_presentation(true, cx);
        if should_reply && let Some(on_reply) = self.on_reply.clone() {
            cx.defer_in(window, move |_, window, cx| on_reply(window, cx));
        }
    }

    fn reset_reply_presentation(&mut self, animated: bool, cx: &mut Context<Self>) {
        if !animated && !self.reply_armed {
            let now = RiseClock::monotonic();
            let settled = |value: &Animated<f32>, target: f32| !value.is_running(now) && *value.target() == target;
            if settled(&self.hosted_translation_x, self.selection_translation_x)
                && settled(&self.reply_hint_alpha, 0.0)
                && settled(&self.reply_hint_scale, 1.0)
            {
                return;
            }
        }
        self.reply_armed = false;
        if animated {
            let spec = spring(0.3, 0.78);
            let now = RiseClock::monotonic();
            self.hosted_translation_x.set_in(self.selection_translation_x, spec, now, cx);
            self.reply_hint_alpha.set_in(0.0, spec, now, cx);
            self.reply_hint_scale.set_in(1.0, spec, now, cx);
        } else {
            self.hosted_translation_x.jump(self.selection_translation_x);
            self.reply_hint_alpha.jump(0.0);
            self.reply_hint_scale.jump(1.0);
        }
        cx.notify();
    }

    fn height_audit_scroll_view(&self) -> Option<Entity<RiseManualListView>> {
        self.height_audit_scroll_view.as_ref().and_then(WeakEntity::upgrade)
    }

    /// true if in a window, the probe holds a frame only while the hosted content is drawn
    fn is_in_window(&self) -> bool {
        self.hosted_view_probe.window_frame().is_some()
    }

    fn is_visible_for_height_audit(&self, cx: &App) -> bool {
        let Some(frame) = self.hosted_view_probe.window_frame() else {
            return false;
        };
        let Some(scroll_view) = self.height_audit_scroll_view() else {
            return false;
        };
        frame.intersects(&scroll_view.read(cx).bounds())
    }

    fn is_height_audit_eligible(&self, generation: u64) -> bool {
        let is_in_window = self.is_in_window();
        if !is_in_window {
            self.did_report_attachment.set(false);
        }
        self.height_audit_generation == generation
            && self.validated_height_audit_generation != Some(generation)
            && is_in_window
            && self.bounds.is_some_and(|bounds| f32::from(bounds.size.width) > 1.0)
            && self.expected_height.is_some()
            && self.on_height_mismatch.is_some()
    }

    fn perform_height_audit(view: &Entity<Self>, cx: &mut App) -> bool {
        let (generation, on_height_mismatch) = {
            let row = view.read(cx);
            (row.height_audit_generation, row.on_height_mismatch.clone())
        };
        let Some(on_height_mismatch) = on_height_mismatch else {
            return true;
        };
        let natural_height = view.update(cx, |row, cx| row.prepared_viewport_natural_height_mismatch(cx));
        if let Some(natural_height) = natural_height {
            on_height_mismatch(f32::from(natural_height), cx);
        }
        view.read(cx).validated_height_audit_generation == Some(generation)
    }

    fn resolved_natural_height(&mut self) -> Option<f32> {
        let trace_token = ChatPerformanceTrace::shared().begin_operation("hosting.audit_measure", HashMap::new());
        let width = self.bounds.map_or(0.0, |bounds| f32::from(bounds.size.width));
        let natural_height = self.refresh_pinned_content_height(width);
        ChatPerformanceTrace::shared().end_operation(trace_token, 1.0, HashMap::new());
        natural_height
    }

    /// fitting algorithm of a measurement host, same rule the mounted row probe reports
    fn natural_height(
        hosted_view: &AnyView,
        width: f32,
        display_scale: f32,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<f32> {
        if !width.is_finite() || width <= 1.0 {
            return None;
        }
        let mut element = hosted_view.clone().into_any_element();
        let raw_height = f32::from(
            element
                .layout_as_root(
                    Size { width: AvailableSpace::Definite(px(width)), height: AvailableSpace::MinContent },
                    window,
                    cx,
                )
                .height,
        );
        Self::pixel_ceiled_height(raw_height, display_scale)
    }

    fn pixel_ceiled_height(raw_height: f32, display_scale: f32) -> Option<f32> {
        if !raw_height.is_finite() || raw_height <= 0.0 || raw_height >= 100_000.0 {
            return None;
        }
        let pixel_scale = if display_scale.is_finite() { display_scale.max(1.0) } else { 1.0 };
        Some((raw_height * pixel_scale).ceil() / pixel_scale)
    }

    fn install(&mut self, root: Option<RiseManualMessageHostingRoot>, cx: &mut Context<Self>) {
        let trace_token = ChatPerformanceTrace::shared().begin_operation(
            "hosting.install",
            HashMap::from([(
                "reusing_content_view".to_owned(),
                if self.hosted_view.is_none() { "0" } else { "1" }.to_owned(),
            )]),
        );
        match self.hosted_view.clone() {
            Some(hosted_view) => hosted_view.update(cx, |content, cx| {
                content.root = root;
                cx.notify();
            }),
            None => {
                let viewport_scope = self.viewport_scope.clone();
                self.hosted_view = Some(cx.new(|_| RiseManualMessageHostingContent { root, viewport_scope }));
            }
        }
        ChatPerformanceTrace::shared().end_operation(trace_token, 0.75, HashMap::new());
    }

    fn did_move_to_window(&mut self, cx: &mut Context<Self>) {
        if self.did_report_attachment.replace(true)
            || !RiseHostedRowHeightAuditLifecyclePolicy::should_schedule_after_attachment(true, self.needs_validation())
        {
            return;
        }
        RiseHostedRowHeightAuditCoordinator::schedule(
            cx.weak_entity(),
            cx.entity_id(),
            self.height_audit_generation,
            RiseHostedRowAuditPolicy::INITIAL_DELAY,
            cx,
        );
    }

    fn layout_subviews(&mut self, bounds: Bounds<Pixels>, scale: f32, cx: &mut Context<Self>) {
        let trace_token = ChatPerformanceTrace::shared().begin_operation("hosting.layout_subviews", HashMap::new());
        self.bounds = Some(bounds);
        self.window_scale = Some(scale);
        self.did_move_to_window(cx);
        let width = f32::from(bounds.size.width);
        let height = f32::from(bounds.size.height);
        if width > 1.0 && self.pinned_content_width.is_some_and(|pinned| (pinned - width).abs() > 0.5) {
            self.refresh_pinned_content_height(width);
        } else if !self.pin_follows_row_height
            && self.pinned_content_height.is_some()
            && height > 1.0
            && self.pinned_row_height.is_some_and(|pinned| (pinned - height).abs() > 0.5)
        {
            self.pinned_row_height = Some(height);
            self.refresh_pinned_content_height(width);
        }
        self.layout_hosted_view(bounds, cx);
        ChatPerformanceTrace::shared().end_operation(
            trace_token,
            0.75,
            HashMap::from([(
                "has_hosted_view".to_owned(),
                if self.hosted_view.is_none() { "0" } else { "1" }.to_owned(),
            )]),
        );
    }

    fn layout_hosted_view(&mut self, bounds: Bounds<Pixels>, cx: &mut Context<Self>) {
        if !RiseListJumpDebug::is_enabled() {
            return;
        }
        let bounds_height = f32::from(bounds.size.height);
        let host_height = if self.pin_follows_row_height {
            bounds_height
        } else {
            self.pinned_content_height.unwrap_or(bounds_height)
        };
        RiseListJumpDebug::record_bubble_layout(
            cx.entity_id(),
            host_height * 0.5,
            0.0,
            host_height,
            bounds_height,
            self.debug_is_own_bubble,
            cx,
        );
    }

    /// takes this frame natural height as the pinned one, none when there is nothing to measure
    fn refresh_pinned_content_height(&mut self, width: f32) -> Option<f32> {
        if self.hosted_view.is_none() || width <= 1.0 {
            return None;
        }
        let natural = self.natural_content_height?;
        self.pinned_content_height = Some(natural);
        self.pinned_content_width = Some(width);
        Some(natural)
    }
}

impl RiseManualListReusableView for RiseManualMessageHostingView {
    fn prepare_for_manual_list_reuse(&mut self, cx: &mut Context<Self>) {
        self.set_animated_media_visible_in_viewport(false, cx);
        self.height_audit_generation = self.height_audit_generation.wrapping_add(1);
        RiseHostedRowHeightAuditCoordinator::cancel(cx.entity_id(), cx);
        self.validated_height_audit_generation = None;
        self.expected_height = None;
        self.last_reported_natural_height = 0.0;
        self.on_height_mismatch = None;
        self.pinned_content_height = None;
        self.pinned_content_width = None;
        self.pinned_row_height = None;
        self.pin_follows_row_height = false;
        self.allows_reply = false;
        self.on_reply = None;
        self.selection_translation_x = 0.0;
        self.on_row_double_tap = None;
        self.row_double_tap_armed = false;
        self.reply_pan_phase = None;
        self.reset_reply_presentation(false, cx);
        self.install(None, cx);
        self.configure_key = None;
    }
}

impl RiseManualListSelectionPresentable for RiseManualMessageHostingView {
    fn apply_manual_list_selection_presentation(
        &mut self,
        is_active: bool,
        content_translation_x: f32,
        cx: &mut Context<Self>,
    ) {
        self.selection_translation_x = if is_active { content_translation_x } else { 0.0 };
        self.reset_reply_presentation(false, cx);
    }
}

impl RiseManualListViewportVisibilityReceiving for RiseManualMessageHostingView {
    fn set_animated_media_visible_in_viewport(&mut self, is_visible: bool, cx: &mut App) {
        if self.viewport_scope.is_visible.get() == is_visible {
            return;
        }
        self.viewport_scope.is_visible.set(is_visible);
        cx.notify(self.entity_id);
        RiseAppEvents::default_center().post(ANIMATED_MEDIA_VIEWPORT_SCOPE_DID_CHANGE);
    }
}

impl AnimatedMediaViewportScopeProviding for RiseManualMessageHostingView {
    fn is_animated_media_visible_in_viewport(&self) -> bool {
        self.viewport_scope.is_visible.get()
    }
}

impl RiseManualListPreparedHeightValidating for RiseManualMessageHostingView {
    fn needs_prepared_viewport_height_validation(&self, _cx: &App) -> bool {
        self.needs_validation()
    }

    fn prepared_viewport_natural_height_mismatch(&mut self, _cx: &mut App) -> Option<Pixels> {
        self.on_height_mismatch.as_ref()?;
        let expected_height = self.expected_height?;
        let natural_height = self.resolved_natural_height()?;
        self.validated_height_audit_generation = Some(self.height_audit_generation);
        let pixel_scale = self.window_scale.filter(|scale| scale.is_finite()).map_or(1.0, |scale| scale.max(1.0));
        let tolerance = (1.0 / pixel_scale).max(0.5);
        if (natural_height - expected_height).abs() <= tolerance
            || (natural_height - self.last_reported_natural_height).abs() <= tolerance
        {
            return None;
        }
        self.last_reported_natural_height = natural_height;
        Some(px(natural_height))
    }
}

impl HoldContextMenuSnapshotRenderRoot for RiseManualMessageHostingView {
    fn hold_context_menu_live_source_view(&self) -> Option<ViewScreenSource> {
        let hosted_view = self.hosted_view.clone()?;
        Some(ViewScreenSource::new(hosted_view, self.hosted_view_probe.clone()))
    }
}

impl Render for RiseManualMessageHostingView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.weak_entity();
        let now = RiseClock::monotonic();
        let translation_x = self.hosted_translation_x.frame_value(now, window);
        let hint_alpha = self.reply_hint_alpha.frame_value(now, window);
        let hint_scale = self.reply_hint_scale.frame_value(now, window);

        let mut row = div()
            .relative()
            .size_full()
            .overflow_hidden()
            .on_scroll_wheel(cx.listener(Self::handle_scroll_wheel))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::handle_row_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::handle_row_mouse_up));
        if let Some(hosted_view) = self.hosted_view.clone() {
            let content_this = this.clone();
            let hosted_any: AnyView = hosted_view.into();
            let lifted_placeholder = HoldContextMenuSourceLift::lifted_placeholder(&hosted_any);
            let is_lifted = lifted_placeholder.is_some();
            let hosted_content = lifted_placeholder.unwrap_or_else(|| hosted_any.clone().into_any_element());
            row = row.child(
                div()
                    .absolute()
                    .top_0()
                    .left(px(translation_x))
                    .w_full()
                    .child(hosted_content)
                    .when(!is_lifted, |slot| {
                        slot.child(measure("rise-manual-message-hosted-content", move |bounds, window, cx| {
                            let natural =
                                Self::pixel_ceiled_height(f32::from(bounds.size.height), window.scale_factor());
                            let _ = content_this.update(cx, |row, _| row.natural_content_height = natural);
                        }))
                    })
                    .child(ViewAnchorProbeReader::new(self.hosted_view_probe.clone())),
            );
        }
        if hint_alpha > 0.0
            && let Some(bounds) = self.bounds
        {
            let side = Self::REPLY_HINT_SIDE;
            let hint_left = if self.reply_pulls_right {
                MessageSizes::row_side_inset()
            } else {
                (f32::from(bounds.size.width) - MessageSizes::row_side_inset() - side).max(0.0)
            };
            let hint_top = ((f32::from(bounds.size.height) - side) * 0.5).max(0.0);
            row = row.child(
                div()
                    .absolute()
                    .left(px(hint_left))
                    .top(px(hint_top))
                    .size(px(side))
                    .flex()
                    .items_center()
                    .justify_center()
                    .opacity(hint_alpha)
                    .children(IconUi::sized(
                        "arrowshape.turn.up.left.fill",
                        px(side * hint_scale),
                        self.reply_hint_tint_color,
                        true,
                    )),
            );
        }
        if self.allows_reply {
            let tracker = this.clone();
            row = row.child(
                canvas(
                    |_, _, _| {},
                    move |_, _, window, _| {
                        let (moves, ups, scrolls) = (tracker.clone(), tracker.clone(), tracker.clone());
                        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                            if phase == DispatchPhase::Bubble {
                                let _ = moves.update(cx, |row, cx| row.handle_reply_mouse_move(event, window, cx));
                            }
                        });
                        window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                            if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
                                let _ = ups.update(cx, |row, cx| row.handle_reply_mouse_release(true, window, cx));
                            }
                        });
                        window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
                            if phase == DispatchPhase::Bubble {
                                let _ = scrolls.update(cx, |row, cx| row.handle_active_reply_scroll(event, window, cx));
                            }
                        });
                    },
                )
                .absolute()
                .size_0(),
            );
        }
        row.child(measure("rise-manual-message-hosting-bounds", move |bounds, window, cx| {
            let scale = window.scale_factor();
            let _ = this.update(cx, |row, cx| row.layout_subviews(bounds, scale, cx));
        }))
    }
}

struct RiseHostedRowIdentity {
    entry_id: String,
}

impl RiseHostedRowIdentity {
    fn body(&self, content: AnyElement) -> AnyElement {
        div().id(ElementId::Name(self.entry_id.clone().into())).child(content).into_any_element()
    }
}
