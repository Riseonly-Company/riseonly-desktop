use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui::{
    AnyView, App, AppContext, Bounds, Context, Edges, Empty, Entity, EntityId, IntoElement, Render, TestAppContext,
    VisualTestContext, Window, point, px, size,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_uuid::Uuid;

use super::rise_manual_list_types::{
    RiseManualListItem, RiseManualListItemMountPolicy, RiseManualListItemProvider, RiseManualListReusableView,
    RiseManualListScrollMetrics, RiseManualListTopStickyHeader, RiseManualListTransaction,
    RiseManualListTransactionAnchor,
};
use super::rise_manual_list_view::{
    RiseManualListDeferredRecyclePolicy, RiseManualListMountEnvelope, RiseManualListMountEnvelopePolicy,
    RiseManualListMountFrame, RiseManualListView,
};
use crate::core::ui::list_engine::rise_manual_list_integration::RiseManualListConformances;
use crate::modules::chat::components::chat::async_render::rise_manual_message_hosting_view::{
    RiseHostedRowAuditPolicy, RiseHostedRowHeightAuditLifecyclePolicy, RiseHostedRowViewportAuditPolicy,
};
use crate::modules::chat::components::chat::async_render::rise_message_list_view::{
    RiseFallbackMeasurementPolicy, RiseMessageFirstFramePresentationPolicy,
    RiseMessageInitialFrameRevealDeliveryPolicy, RiseMessageInitialFrameRevealDeliveryPolicyAction,
    RiseMessageInitialScrollPosition, RiseMessageInitialViewportRecoveryPolicy,
    RiseMessageScrollPositionPersistencePolicy,
};

const DRAIN_MAXIMUM_BATCHES: usize = 128;

struct TestRow;

impl Render for TestRow {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

#[derive(Default)]
struct RiseManualListReuseProbeView {
    preparation_count: usize,
}

impl Render for RiseManualListReuseProbeView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

impl RiseManualListReusableView for RiseManualListReuseProbeView {
    fn prepare_for_manual_list_reuse(&mut self, _cx: &mut Context<Self>) {
        self.preparation_count += 1;
    }
}

struct ListHarness<'a> {
    list: Entity<RiseManualListView>,
    cx: &'a mut VisualTestContext,
}

impl<'a> ListHarness<'a> {
    fn new(cx: &'a mut TestAppContext, height: f32) -> Self {
        let (list, cx) = cx.add_window_view(|window, cx| RiseManualListView::new(window, cx));
        cx.simulate_resize(size(px(320.0), px(height)));
        let mut harness = Self { list, cx };
        let frame = Bounds::new(point(px(0.0), px(0.0)), size(px(320.0), px(height)));
        harness.update(|list, window, cx| list.layout_subviews(frame, window, cx));
        harness
    }

    fn update<R>(
        &mut self,
        f: impl FnOnce(&mut RiseManualListView, &mut Window, &mut Context<RiseManualListView>) -> R,
    ) -> R {
        let list = self.list.clone();
        self.cx.update(|window, app| list.update(app, |list, cx| f(list, window, cx)))
    }

    fn set_preload(&mut self, value: f32) {
        self.update(|list, window, cx| list.set_preload_distance(px(value), window, cx));
    }

    fn enqueue(&mut self, transaction: RiseManualListTransaction) {
        self.update(|list, window, cx| list.enqueue(transaction, None, None, window, cx));
    }

    fn set_scrolling(&mut self, is_scrolling: bool) {
        self.update(|list, window, cx| list.set_user_scrolling_override_for_testing(Some(is_scrolling), window, cx));
    }

    fn set_active_offset(&mut self, value: f32) -> bool {
        self.update(|list, window, cx| list.set_active_scroll_content_offset_y_for_testing(px(value), window, cx))
    }

    fn set_offset(&mut self, value: f32) -> bool {
        self.update(|list, window, cx| list.set_content_offset_y(px(value), false, true, window, cx))
    }

    fn drain(&mut self, maximum_batches: usize) {
        self.update(|list, window, cx| list.drain_deferred_mounting_for_testing(maximum_batches, window, cx));
    }

    fn visible_ids(&mut self) -> Vec<String> {
        self.update(|list, _, _| list.visible_ids().to_vec())
    }

    fn is_mounted(&mut self, id: &str) -> bool {
        self.update(|list, _, _| list.is_item_mounted_for_testing(id))
    }

    fn is_warm(&mut self, id: &str) -> bool {
        self.update(|list, _, _| list.is_item_warm_mounted_for_testing(id))
    }

    fn is_placeholder(&mut self, id: &str) -> bool {
        self.update(|list, _, _| list.is_item_placeholder_for_testing(id))
    }

    fn scheduled_generation(&mut self) -> Option<u64> {
        self.update(|list, _, _| list.scheduled_deferred_mount_generation_for_testing())
    }

    fn perform_batch(&mut self, generation: u64, frame: Option<RiseManualListMountFrame>) {
        self.update(|list, window, cx| list.perform_deferred_mount_batch_for_testing(generation, frame, window, cx));
    }

    fn sync_passes(&mut self) -> usize {
        self.update(|list, _, _| list.visible_synchronization_pass_count_for_testing())
    }

    fn mounted_view_id(&mut self, id: &str) -> Option<EntityId> {
        self.update(|list, _, _| list.mounted_view_for_testing(id).map(|view| view.entity_id()))
    }

    fn mounted_min_y(&mut self, id: &str) -> f32 {
        self.update(|list, _, _| {
            list.mounted_frame_in_viewport_for_testing(id).map_or(-1.0, |frame| f32::from(frame.origin.y))
        })
    }

    fn is_top_sticky(&mut self, id: &str) -> bool {
        self.update(|list, _, _| list.is_item_top_sticky_for_testing(id))
    }
}

fn prefixed_ids(prefix: &str, count: usize) -> Vec<String> {
    (0..count).map(|index| format!("{prefix}-{index}")).collect()
}

fn ids(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

fn close(value: f32, expected: f32) -> bool {
    (value - expected).abs() <= 0.5
}

fn deferred_item(
    id: &str,
    height: f32,
    reuse_identifier: &str,
    mount_policy: RiseManualListItemMountPolicy,
    top_sticky_header: Option<RiseManualListTopStickyHeader>,
    on_factory: Rc<dyn Fn(&str)>,
) -> RiseManualListItem {
    let id = id.to_string();
    RiseManualListItem::new(
        height,
        reuse_identifier.to_string(),
        1,
        mount_policy,
        top_sticky_header,
        None,
        None,
        None,
        Rc::new(move |reusable: Option<AnyView>, _: &mut Window, cx: &mut App| {
            on_factory(&id);
            reusable.unwrap_or_else(|| cx.new(|_| TestRow).into())
        }),
    )
}

fn provider(
    height: f32,
    reuse_identifier: &'static str,
    mount_policy: RiseManualListItemMountPolicy,
    on_factory: Rc<dyn Fn(&str)>,
) -> RiseManualListItemProvider {
    Rc::new(move |id: &str, _: f32, _: &mut Window, _: &mut App| {
        deferred_item(id, height, reuse_identifier, mount_policy, None, on_factory.clone())
    })
}

fn deferred_provider(reuse_identifier: &'static str) -> RiseManualListItemProvider {
    provider(60.0, reuse_identifier, RiseManualListItemMountPolicy::DeferredDuringFastScroll, Rc::new(|_: &str| {}))
}

fn immediate_provider(reuse_identifier: &'static str) -> RiseManualListItemProvider {
    provider(60.0, reuse_identifier, RiseManualListItemMountPolicy::Immediate, Rc::new(|_: &str| {}))
}

fn transaction(
    ids: Vec<String>,
    anchor: RiseManualListTransactionAnchor,
    updated_ids: Option<HashSet<String>>,
    provider: RiseManualListItemProvider,
) -> RiseManualListTransaction {
    RiseManualListTransaction::new(ids, anchor, None, updated_ids, None, None, None, false, HashSet::new(), provider)
}

fn top(ids: Vec<String>, provider: RiseManualListItemProvider) -> RiseManualListTransaction {
    transaction(ids, RiseManualListTransactionAnchor::Top, None, provider)
}

fn counter() -> (Rc<Cell<usize>>, Rc<dyn Fn(&str)>) {
    let count = Rc::new(Cell::new(0usize));
    let on_factory: Rc<dyn Fn(&str)> = {
        let count = count.clone();
        Rc::new(move |_: &str| count.set(count.get() + 1))
    };
    (count, on_factory)
}

fn counter_by_id() -> (Rc<RefCell<HashMap<String, usize>>>, Rc<dyn Fn(&str)>) {
    let counts: Rc<RefCell<HashMap<String, usize>>> = Rc::new(RefCell::new(HashMap::new()));
    let on_factory: Rc<dyn Fn(&str)> = {
        let counts = counts.clone();
        Rc::new(move |id: &str| *counts.borrow_mut().entry(id.to_string()).or_insert(0) += 1)
    };
    (counts, on_factory)
}

fn count_of(counts: &Rc<RefCell<HashMap<String, usize>>>, id: &str) -> Option<usize> {
    counts.borrow().get(id).copied()
}

#[test]
fn fallback_geometry_prepares_exact_bounded_history_before_first_commit() {
    assert_eq!(RiseFallbackMeasurementPolicy::nearby_range(0..320, 319), 272..320);
    assert_eq!(RiseFallbackMeasurementPolicy::nearby_range(0..320, 0), 0..48);
    let middle = RiseFallbackMeasurementPolicy::nearby_range(100..180, 140);
    assert_eq!(middle.end - middle.start, RiseFallbackMeasurementPolicy::NEARBY_ENTRY_LIMIT);
    assert!(middle.contains(&140));

    let bottom = RiseFallbackMeasurementPolicy::initial_preparation_range(320);
    assert_eq!(bottom, 0..320);

    let centered = RiseFallbackMeasurementPolicy::initial_preparation_range(320);
    assert_eq!(centered, 0..320);
}

#[test]
fn first_frame_range_does_not_install_estimated_rows_for_dense_history() {
    let range = RiseFallbackMeasurementPolicy::initial_preparation_range(320);
    assert_eq!(range, 0..320);

    assert_eq!(RiseFallbackMeasurementPolicy::guarded_visible_range(100..112, 320), 92..120);
    assert_eq!(RiseFallbackMeasurementPolicy::guarded_visible_range(0..5, 10), 0..10);
}

#[test]
fn exact_geometry_scope_is_bounded_only_for_initial_and_deferred_commits() {
    let prepared = 120..168;
    assert_eq!(
        RiseFallbackMeasurementPolicy::exact_geometry_range_for_commit(320, prepared.clone(), false, false),
        prepared
    );
    assert_eq!(
        RiseFallbackMeasurementPolicy::exact_geometry_range_for_commit(320, prepared.clone(), true, true),
        prepared
    );
    assert_eq!(RiseFallbackMeasurementPolicy::exact_geometry_range_for_commit(320, prepared, true, false), 0..320);
}

#[test]
fn first_frame_presentation_epoch_distinguishes_repeated_open_of_same_chat() {
    let first_open = Uuid::new_v4();
    let second_open = Uuid::new_v4();

    assert!(!RiseMessageFirstFramePresentationPolicy::starts_new_presentation(
        Some(first_open),
        Some(first_open),
        false
    ));
    assert!(RiseMessageFirstFramePresentationPolicy::starts_new_presentation(
        Some(first_open),
        Some(second_open),
        false
    ));
    assert!(RiseMessageFirstFramePresentationPolicy::starts_new_presentation(None, None, true));
    assert!(RiseMessageFirstFramePresentationPolicy::should_supersede_unrevealed_owner(10, 11, true));
    assert!(!RiseMessageFirstFramePresentationPolicy::should_supersede_unrevealed_owner(10, 10, true));
    assert!(!RiseMessageFirstFramePresentationPolicy::should_supersede_unrevealed_owner(10, 11, false));
    assert!(RiseMessageFirstFramePresentationPolicy::pins_initial_position_to_bottom(
        RiseMessageInitialScrollPosition::Bottom,
        false
    ));
    assert!(!RiseMessageFirstFramePresentationPolicy::pins_initial_position_to_bottom(
        RiseMessageInitialScrollPosition::Bottom,
        true
    ));
    assert!(!RiseMessageInitialViewportRecoveryPolicy::should_reveal_applied_geometry(1));

    assert!(RiseMessageScrollPositionPersistencePolicy::discards_saved_position(0.0, true));
    assert!(RiseMessageScrollPositionPersistencePolicy::discards_saved_position(80.0, false));
    assert!(!RiseMessageScrollPositionPersistencePolicy::discards_saved_position(80.0, true));
    assert!(RiseMessageInitialViewportRecoveryPolicy::should_reveal_applied_geometry(
        RiseMessageInitialViewportRecoveryPolicy::MAXIMUM_STABLE_SNAPSHOT_RETRIES
    ));

    assert_eq!(
        RiseMessageInitialFrameRevealDeliveryPolicy::action(true, false),
        RiseMessageInitialFrameRevealDeliveryPolicyAction::DeferUntilWindowAttachment
    );
    assert_eq!(
        RiseMessageInitialFrameRevealDeliveryPolicy::action(true, true),
        RiseMessageInitialFrameRevealDeliveryPolicyAction::Deliver
    );
    assert_eq!(
        RiseMessageInitialFrameRevealDeliveryPolicy::action(false, true),
        RiseMessageInitialFrameRevealDeliveryPolicyAction::DiscardStale
    );
}

#[test]
#[allow(clippy::assertions_on_constants)]
fn hosted_viewport_audit_validates_every_visible_fallback_before_reveal() {
    let indices = RiseHostedRowViewportAuditPolicy::prioritized_indices(10..20);
    assert_eq!(indices.first(), Some(&15));
    assert_eq!(indices.iter().copied().collect::<HashSet<_>>(), (10..20).collect::<HashSet<_>>());
    assert_eq!(RiseHostedRowViewportAuditPolicy::MAXIMUM_MEASUREMENTS_PER_PASS, usize::MAX);
    assert!(RiseHostedRowAuditPolicy::MAX_AUDITS_PER_TURN <= 3);
    assert_eq!(RiseHostedRowAuditPolicy::MAX_VISIBLE_AUDITS_PER_SCROLLING_TURN, 1);
    assert!(RiseHostedRowHeightAuditLifecyclePolicy::should_schedule_after_attachment(true, true));
    assert!(!RiseHostedRowHeightAuditLifecyclePolicy::should_schedule_after_attachment(false, true));
}

#[test]
fn mount_envelope_keeps_several_frames_of_hosted_lookahead() {
    let preload = RiseManualListMountEnvelopePolicy::default_preload_distance(px(844.0));
    assert!(preload >= px(360.0));
    assert!(preload <= px(560.0));

    let envelope = RiseManualListMountEnvelopePolicy::resolve(preload, px(844.0), 12_000.0);
    assert!(envelope.immediate_ahead_distance >= px(180.0));
    assert!(envelope.immediate_ahead_distance <= px(460.0));
    assert!(envelope.immediate_behind_distance >= px(120.0));
    assert!(envelope.immediate_behind_distance <= px(460.0));
    assert!(envelope.ahead_distance <= preload + px(844.0 * 0.72));

    let disabled = RiseManualListMountEnvelopePolicy::resolve(px(0.0), px(844.0), 12_000.0);
    assert_eq!(
        disabled,
        RiseManualListMountEnvelope {
            ahead_distance: px(0.0),
            behind_distance: px(0.0),
            immediate_ahead_distance: px(0.0),
            immediate_behind_distance: px(0.0),
        }
    );
}

#[test]
fn deferred_mount_batch_bounds_warming_during_motion_and_at_rest() {
    let dragging = RiseManualListMountEnvelopePolicy::deferred_mount_batch(12_000.0, 120, px(60.0), false);
    let decelerating = RiseManualListMountEnvelopePolicy::deferred_mount_batch(12_000.0, 60, px(60.0), false);
    let idle = RiseManualListMountEnvelopePolicy::deferred_mount_batch(0.0, 120, px(60.0), true);

    assert_eq!(dragging.maximum_mounts, 1);
    assert!(dragging.time_budget > 0.0 && dragging.time_budget < 1.0 / 120.0);
    assert_eq!(decelerating.maximum_mounts, 1);
    assert!(decelerating.time_budget > 0.0 && decelerating.time_budget < 1.0 / 60.0);
    assert_eq!(idle.maximum_mounts, 2);
    assert!(idle.time_budget < 1.0 / 120.0);
    assert_eq!(RiseManualListMountEnvelopePolicy::MAXIMUM_FALLBACK_HOSTED_MOUNTS_PER_IDLE_BATCH, 1);
    assert!(RiseManualListMountEnvelopePolicy::is_fallback_hosted_row("RiseManualMessageHostingView.message.photo"));
    assert!(!RiseManualListMountEnvelopePolicy::is_fallback_hosted_row("RiseNativeTextMessageCell"));
}

#[test]
fn deferred_mount_uses_actual_cadence_and_rejects_late_first_mount() {
    for fps in [30_usize, 60, 80, 90, 120] {
        let frame = RiseManualListMountFrame::new(10.0, 10.0 + 1.0 / fps as f64).expect("frame");
        assert_eq!(frame.frames_per_second(), fps);
        assert!(frame.can_start_mount(10.000_1, 0.002));
        assert!(!frame.can_start_mount(frame.target_timestamp - 0.000_1, 0.000_2));
        assert!(!frame.can_start_mount(frame.work_deadline() - 0.001, 0.002));
        assert!(frame.can_start_mount(10.000_1, 0.020));
    }
    assert_eq!(RiseManualListMountFrame::new(1.0, 1.0), None);
    assert_eq!(RiseManualListMountFrame::new(f64::NAN, 2.0), None);
    assert_eq!(RiseManualListMountFrame::new(1.0, f64::INFINITY), None);
}

#[gpui::test]
fn late_frame_preserves_queued_row_and_detached_list_does_not_tick(cx: &mut TestAppContext) {
    let mut list = ListHarness::new(cx, 180.0);
    list.set_preload(600.0);
    let (factory_calls, on_factory) = counter();
    list.enqueue(top(
        prefixed_ids("frame-budget", 30),
        provider(
            60.0,
            "RiseManualMessageHostingView.test",
            RiseManualListItemMountPolicy::DeferredDuringFastScroll,
            on_factory,
        ),
    ));
    let generation = list.scheduled_generation().expect("scheduled generation");
    assert!(!list.update(|list, _, _| list.has_deferred_mount_display_link_for_testing()));
    let calls_before_late_frame = factory_calls.get();
    let now = RiseClock::monotonic().as_secs_f64();
    let late_frame = RiseManualListMountFrame::new(now - 1.0 / 60.0, now).expect("late frame");
    list.perform_batch(generation, Some(late_frame));
    assert_eq!(factory_calls.get(), calls_before_late_frame);
    assert!(list.scheduled_generation().is_some());
    list.drain(DRAIN_MAXIMUM_BATCHES);
    assert!(factory_calls.get() > calls_before_late_frame);
    assert_eq!(list.scheduled_generation(), None);
    assert!(!list.update(|list, _, _| list.has_deferred_mount_display_link_for_testing()));
}

#[test]
#[allow(clippy::assertions_on_constants)]
fn deferred_recycle_waits_for_a_continuous_idle_window() {
    assert!(RiseManualListDeferredRecyclePolicy::QUIET_PERIOD >= 0.45);
    assert_eq!(RiseManualListDeferredRecyclePolicy::MAXIMUM_IDENTITY_CACHED_ITEMS, 24);
    assert_eq!(RiseManualListDeferredRecyclePolicy::PREFERRED_GENERIC_OVERFLOW_LIMIT, 16);
    assert_eq!(RiseManualListDeferredRecyclePolicy::MAXIMUM_RECYCLES_PER_BATCH, 1);
    assert!(RiseManualListDeferredRecyclePolicy::PRESERVES_BOUNDED_IDENTITY_CACHE_WHILE_IDLE);
    assert!(RiseManualListDeferredRecyclePolicy::should_defer_preparation(true));
    assert!(!RiseManualListDeferredRecyclePolicy::should_defer_preparation(false));
    assert!(RiseManualListDeferredRecyclePolicy::remaining_quiet_time(10.0, 10.2) > 0.0);
    assert_eq!(RiseManualListDeferredRecyclePolicy::remaining_quiet_time(10.0, 10.6), 0.0);
}

#[gpui::test]
fn deferred_rows_initially_use_placeholders_only_in_immediate_envelope(cx: &mut TestAppContext) {
    let mut list = ListHarness::new(cx, 180.0);
    list.set_preload(600.0);
    list.enqueue(top(prefixed_ids("hosted", 30), deferred_provider("hosted-envelope")));

    assert!(list.is_mounted("hosted-0"));
    assert!(list.is_warm("hosted-0"));
    assert!(list.is_mounted("hosted-4"));
    assert!(list.is_mounted("hosted-5"));
    assert!(list.is_placeholder("hosted-5"));
    assert!(!list.is_mounted("hosted-6"));
    assert!(!list.is_mounted("hosted-10"));
}

#[gpui::test]
fn idle_queue_warm_configures_hosted_rows_across_the_outer_envelope(cx: &mut TestAppContext) {
    let mut list = ListHarness::new(cx, 180.0);
    list.set_preload(600.0);
    list.enqueue(top(prefixed_ids("warm-hosted", 30), deferred_provider("warm-hosted-envelope")));

    assert!(list.is_placeholder("warm-hosted-5"));
    assert!(!list.is_mounted("warm-hosted-10"));

    list.drain(DRAIN_MAXIMUM_BATCHES);

    assert!(list.is_warm("warm-hosted-5"));
    assert!(list.is_warm("warm-hosted-10"));
    assert!(!list.is_mounted("warm-hosted-14"));
}

#[gpui::test]
fn active_scroll_inside_warm_envelope_preserves_configured_rows(cx: &mut TestAppContext) {
    let mut list = ListHarness::new(cx, 180.0);
    list.set_preload(600.0);
    let (view_factory_calls, on_factory) = counter();
    list.enqueue(top(
        prefixed_ids("warm-scroll", 30),
        provider(60.0, "warm-scroll-envelope", RiseManualListItemMountPolicy::DeferredDuringFastScroll, on_factory),
    ));
    list.drain(DRAIN_MAXIMUM_BATCHES);
    let synchronization_passes = list.sync_passes();
    let factory_calls_before_scroll = view_factory_calls.get();

    list.set_scrolling(true);
    assert!(list.set_active_offset(240.0));

    assert_eq!(list.visible_ids(), ids(&["warm-scroll-4", "warm-scroll-5", "warm-scroll-6"]));
    assert_eq!(list.sync_passes(), synchronization_passes);
    assert_eq!(view_factory_calls.get(), factory_calls_before_scroll);
    assert!(list.is_warm("warm-scroll-6"));
    list.set_scrolling(false);
}

#[gpui::test]
fn active_scroll_reports_metrics_before_general_scroll_work(cx: &mut TestAppContext) {
    let mut list = ListHarness::new(cx, 180.0);
    list.enqueue(top(prefixed_ids("metrics", 10), immediate_provider("default")));

    let deliveries: Rc<RefCell<Vec<&'static str>>> = Rc::new(RefCell::new(Vec::new()));
    let active = deliveries.clone();
    let general = deliveries.clone();
    list.update(|list, _, _| {
        list.on_active_scroll_metrics_changed =
            Some(Rc::new(move |_: &RiseManualListScrollMetrics, _: &mut Window, _: &mut App| {
                active.borrow_mut().push("active");
            }));
        list.on_scroll_metrics_changed =
            Some(Rc::new(move |_: &RiseManualListScrollMetrics, _: &mut Window, _: &mut App| {
                general.borrow_mut().push("general");
            }));
    });
    list.set_scrolling(true);

    assert!(list.set_active_offset(240.0));
    assert_eq!(*deliveries.borrow(), vec!["active", "general"]);

    list.set_scrolling(false);
}

#[gpui::test]
fn quick_scroll_reversal_reattaches_the_exact_hosted_rows(cx: &mut TestAppContext) {
    let mut list = ListHarness::new(cx, 180.0);
    list.set_preload(0.0);

    let row_ids = prefixed_ids("identity-hosted", 20);
    let (factory_calls_by_id, on_factory) = counter_by_id();
    let hosted = provider(
        60.0,
        "RiseManualMessageHostingView.identity-test",
        RiseManualListItemMountPolicy::DeferredDuringFastScroll,
        on_factory,
    );
    list.enqueue(top(row_ids.clone(), hosted.clone()));

    assert_eq!(count_of(&factory_calls_by_id, "identity-hosted-0"), Some(1));
    assert_eq!(count_of(&factory_calls_by_id, "identity-hosted-1"), Some(1));
    assert_eq!(count_of(&factory_calls_by_id, "identity-hosted-2"), Some(1));

    list.set_scrolling(true);
    assert!(list.set_active_offset(600.0));
    assert_eq!(count_of(&factory_calls_by_id, "identity-hosted-10"), Some(1));
    assert_eq!(count_of(&factory_calls_by_id, "identity-hosted-11"), Some(1));
    assert_eq!(count_of(&factory_calls_by_id, "identity-hosted-12"), Some(1));
    let (identity, generic) = list.update(|list, _, _| list.deferred_recycle_counts_for_testing());
    assert!(identity >= 3);
    assert_eq!(generic, 0);

    assert!(list.set_active_offset(0.0));
    assert_eq!(count_of(&factory_calls_by_id, "identity-hosted-0"), Some(1));
    assert_eq!(count_of(&factory_calls_by_id, "identity-hosted-1"), Some(1));
    assert_eq!(count_of(&factory_calls_by_id, "identity-hosted-2"), Some(1));

    assert!(list.set_active_offset(600.0));
    list.set_scrolling(false);
    list.enqueue(transaction(
        row_ids,
        RiseManualListTransactionAnchor::PreserveVisible,
        Some(HashSet::from(["identity-hosted-0".to_string()])),
        hosted,
    ));
    list.set_scrolling(true);
    assert!(list.set_active_offset(0.0));
    assert_eq!(count_of(&factory_calls_by_id, "identity-hosted-0"), Some(2));
    list.set_scrolling(false);
}

#[gpui::test]
fn idle_snapshot_removal_does_not_synchronously_reset_hosted_rows(cx: &mut TestAppContext) {
    let mut list = ListHarness::new(cx, 180.0);
    list.cx.update(|_, app| RiseManualListConformances::register_reusable_view::<RiseManualListReuseProbeView>(app));
    list.set_preload(0.0);

    let hosted_views: Rc<RefCell<Vec<Entity<RiseManualListReuseProbeView>>>> = Rc::new(RefCell::new(Vec::new()));
    let probe_provider: RiseManualListItemProvider = {
        let hosted_views = hosted_views.clone();
        Rc::new(move |_: &str, _: f32, _: &mut Window, _: &mut App| {
            let hosted_views = hosted_views.clone();
            RiseManualListItem::new(
                60.0,
                "RiseManualMessageHostingView.idle-removal".to_string(),
                1,
                RiseManualListItemMountPolicy::DeferredDuringFastScroll,
                None,
                None,
                None,
                None,
                Rc::new(move |_: Option<AnyView>, _: &mut Window, cx: &mut App| {
                    let view = cx.new(|_| RiseManualListReuseProbeView::default());
                    hosted_views.borrow_mut().push(view.clone());
                    view.into()
                }),
            )
        })
    };
    list.enqueue(top(ids(&["idle-hosted-0", "idle-hosted-1", "idle-hosted-2"]), probe_provider));
    assert_eq!(hosted_views.borrow().len(), 3);

    let empty_provider: RiseManualListItemProvider =
        Rc::new(|_: &str, _: f32, _: &mut Window, _: &mut App| -> RiseManualListItem {
            panic!("An empty snapshot must not request row content")
        });
    list.enqueue(transaction(Vec::new(), RiseManualListTransactionAnchor::Top, Some(HashSet::new()), empty_provider));

    let views = hosted_views.borrow().clone();
    assert!(list.cx.update(|_, app| views.iter().all(|view| view.read(app).preparation_count == 0)));
}

#[gpui::test]
fn same_version_factory_refresh_survives_unmount_inside_transaction_finish(cx: &mut TestAppContext) {
    let mut list = ListHarness::new(cx, 180.0);
    list.set_preload(0.0);

    let row_ids = prefixed_ids("finish-refresh", 10);
    let (factory_calls_by_id, on_factory) = counter_by_id();
    let hosted = provider(
        60.0,
        "RiseManualMessageHostingView.finish-refresh",
        RiseManualListItemMountPolicy::DeferredDuringFastScroll,
        on_factory,
    );

    list.enqueue(top(row_ids.clone(), hosted.clone()));
    assert_eq!(count_of(&factory_calls_by_id, "finish-refresh-0"), Some(1));

    list.enqueue(transaction(
        row_ids.clone(),
        RiseManualListTransactionAnchor::Bottom,
        Some(HashSet::from(["finish-refresh-0".to_string()])),
        hosted.clone(),
    ));
    assert!(!list.is_mounted("finish-refresh-0"));

    list.enqueue(transaction(row_ids, RiseManualListTransactionAnchor::Top, Some(HashSet::new()), hosted));
    assert_eq!(count_of(&factory_calls_by_id, "finish-refresh-0"), Some(2));
}

#[gpui::test]
fn active_scroll_cancels_stale_idle_batch_and_resumes_warming_at_rest(cx: &mut TestAppContext) {
    let mut list = ListHarness::new(cx, 180.0);
    list.set_preload(600.0);
    let (configured_ids, on_factory) = counter();
    list.enqueue(top(
        prefixed_ids("paused-hosted", 30),
        provider(
            60.0,
            "RiseManualMessageHostingView.test",
            RiseManualListItemMountPolicy::DeferredDuringFastScroll,
            on_factory,
        ),
    ));

    let stale_generation = list.scheduled_generation().expect("scheduled generation");
    let calls_before_gesture = configured_ids.get();
    assert!(list.is_placeholder("paused-hosted-5"));

    list.set_scrolling(true);
    assert_eq!(list.scheduled_generation(), None);

    list.perform_batch(stale_generation, None);
    list.drain(DRAIN_MAXIMUM_BATCHES);
    assert_eq!(configured_ids.get(), calls_before_gesture);
    assert!(list.is_placeholder("paused-hosted-5"));

    assert!(list.set_active_offset(300.0));
    assert_eq!(list.visible_ids(), ids(&["paused-hosted-5", "paused-hosted-6", "paused-hosted-7"]));
    assert!(list.is_warm("paused-hosted-5"));
    assert!(list.is_warm("paused-hosted-6"));
    assert!(list.is_warm("paused-hosted-7"));
    let calls_after_visible_synchronization = configured_ids.get();

    list.perform_batch(stale_generation, None);
    assert_eq!(configured_ids.get(), calls_after_visible_synchronization);

    list.set_scrolling(false);
    assert!(list.scheduled_generation().is_some());
    list.drain(DRAIN_MAXIMUM_BATCHES);

    assert!(list.is_warm("paused-hosted-10"));
    assert!(configured_ids.get() > calls_after_visible_synchronization);
}

#[gpui::test]
fn idle_batch_installs_one_fallback_host_and_still_uses_native_capacity(cx: &mut TestAppContext) {
    let mut list = ListHarness::new(cx, 180.0);
    list.set_preload(600.0);

    let fallback_factory_calls = Rc::new(Cell::new(0usize));
    let native_factory_calls = Rc::new(Cell::new(0usize));
    let mixed: RiseManualListItemProvider = {
        let fallback_factory_calls = fallback_factory_calls.clone();
        let native_factory_calls = native_factory_calls.clone();
        Rc::new(move |id: &str, _: f32, _: &mut Window, _: &mut App| {
            let index: usize = id.rsplit('-').next().and_then(|value| value.parse().ok()).unwrap_or(0);
            let is_fallback_hosted = index % 2 != 0;
            let calls = if is_fallback_hosted { fallback_factory_calls.clone() } else { native_factory_calls.clone() };
            deferred_item(
                id,
                60.0,
                if is_fallback_hosted { "RiseManualMessageHostingView.test" } else { "native-deferred-test" },
                RiseManualListItemMountPolicy::DeferredDuringFastScroll,
                None,
                Rc::new(move |_: &str| calls.set(calls.get() + 1)),
            )
        })
    };
    list.enqueue(top(prefixed_ids("mixed-row", 20), mixed));

    let fallback_calls_before_batch = fallback_factory_calls.get();
    let native_calls_before_batch = native_factory_calls.get();

    list.drain(1);

    assert_eq!(fallback_factory_calls.get(), fallback_calls_before_batch + 1);
    assert_eq!(native_factory_calls.get(), native_calls_before_batch + 1);
    assert!(list.is_warm("mixed-row-3"));
    assert!(list.is_warm("mixed-row-4"));
}

#[gpui::test]
fn binary_visible_range_preserves_spacing_and_boundary_semantics(cx: &mut TestAppContext) {
    let mut list = ListHarness::new(cx, 100.0);
    list.update(|list, window, cx| list.set_inter_item_spacing(px(4.0), window, cx));

    let heights: HashMap<&'static str, f32> = HashMap::from([("a", 40.0), ("b", 70.0), ("c", 30.0), ("d", 80.0)]);
    let by_height: RiseManualListItemProvider = Rc::new(move |id: &str, _: f32, _: &mut Window, _: &mut App| {
        deferred_item(
            id,
            heights.get(id).copied().unwrap_or(1.0),
            "range-boundary",
            RiseManualListItemMountPolicy::Immediate,
            None,
            Rc::new(|_: &str| {}),
        )
    });
    list.enqueue(top(ids(&["a", "b", "c", "d"]), by_height));

    assert_eq!(list.visible_ids(), ids(&["a", "b"]));
    assert!(list.set_offset(40.0));
    assert_eq!(list.visible_ids(), ids(&["b", "c"]));
    assert!(list.set_offset(114.0));
    assert_eq!(list.visible_ids(), ids(&["c", "d"]));
}

#[gpui::test]
fn top_sticky_dates_move_the_original_rows_at_the_chrome_boundary(cx: &mut TestAppContext) {
    let mut list = ListHarness::new(cx, 180.0);
    list.set_preload(0.0);
    let edges = Edges { top: px(30.0), right: px(0.0), bottom: px(0.0), left: px(0.0) };
    list.update(|list, window, cx| list.set_viewport_insets(edges, window, cx));
    list.update(|list, window, cx| list.set_top_sticky_viewport_inset(Some(px(20.0)), window, cx));

    let first_date_id = "date-1";
    let second_date_id = "date-2";
    let row_ids = ids(&[
        first_date_id,
        "message-1",
        "message-2",
        second_date_id,
        "message-3",
        "message-4",
        "message-5",
        "message-6",
    ]);
    let (factory_calls_by_id, on_factory) = counter_by_id();
    let dated: RiseManualListItemProvider = Rc::new(move |id: &str, _: f32, _: &mut Window, _: &mut App| {
        let is_date = id == first_date_id || id == second_date_id;
        deferred_item(
            id,
            if is_date { 40.0 } else { 60.0 },
            if is_date { "date" } else { "message" },
            RiseManualListItemMountPolicy::Immediate,
            is_date.then_some(RiseManualListTopStickyHeader),
            on_factory.clone(),
        )
    });
    list.enqueue(top(row_ids, dated));

    let original_first_date_view = list.mounted_view_id(first_date_id);
    assert!(original_first_date_view.is_some());
    assert_eq!(count_of(&factory_calls_by_id, first_date_id), Some(1));

    list.set_scrolling(true);
    assert!(list.set_active_offset(20.0));
    assert!(list.is_top_sticky(first_date_id));
    assert_eq!(list.mounted_view_id(first_date_id), original_first_date_view);
    assert!(close(list.mounted_min_y(first_date_id), 20.0));
    assert_eq!(count_of(&factory_calls_by_id, first_date_id), Some(1));

    assert!(list.set_active_offset(150.0));
    assert!(list.is_top_sticky(first_date_id));
    assert!(list.is_top_sticky(second_date_id));
    assert_eq!(list.mounted_view_id(first_date_id), original_first_date_view);
    assert!(close(list.mounted_min_y(first_date_id), 0.0));
    assert!(close(list.mounted_min_y(second_date_id), 40.0));
    assert_eq!(count_of(&factory_calls_by_id, first_date_id), Some(1));
    assert_eq!(count_of(&factory_calls_by_id, second_date_id), Some(1));
    list.set_scrolling(false);
}

#[gpui::test]
fn one_offset_uses_one_visible_range_and_no_redundant_synchronization_pass(cx: &mut TestAppContext) {
    let mut list = ListHarness::new(cx, 180.0);
    list.set_preload(0.0);
    list.enqueue(top(prefixed_ids("row", 12), immediate_provider("hot-path")));

    let range_computations = list.update(|list, _, _| list.visible_item_range_computation_count_for_testing());
    let synchronization_passes = list.sync_passes();

    assert!(list.set_offset(1.0));
    assert_eq!(
        list.update(|list, _, _| list.visible_item_range_computation_count_for_testing()),
        range_computations + 1
    );
    assert_eq!(list.sync_passes(), synchronization_passes);
    assert_eq!(list.visible_ids(), ids(&["row-0", "row-1", "row-2", "row-3"]));
}

#[gpui::test]
fn reuse_pool_counter_stays_exact_across_mount_churn_and_trimming(cx: &mut TestAppContext) {
    let mut list = ListHarness::new(cx, 180.0);
    list.set_preload(0.0);
    list.update(|list, _, _| list.set_reuse_pool_limit(2));
    list.enqueue(top(prefixed_ids("row", 80), immediate_provider("reuse-counter")));

    assert!(list.update(|list, window, cx| list.scroll_to_bottom(false, window, cx)));
    assert!(list.set_offset(0.0));

    let (tracked, stored) = list.update(|list, _, _| list.reusable_view_counts_for_testing());
    assert_eq!(tracked, stored);
    assert!(tracked <= 2);

    list.update(|list, _, _| list.set_reuse_pool_limit(0));
    let (tracked, stored) = list.update(|list, _, _| list.reusable_view_counts_for_testing());
    assert_eq!(tracked, 0);
    assert_eq!(stored, 0);
}
