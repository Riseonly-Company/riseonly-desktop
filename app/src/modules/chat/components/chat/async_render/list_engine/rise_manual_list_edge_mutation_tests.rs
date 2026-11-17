use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui::{
    AnyView, App, AppContext, Bounds, Context, Edges, Empty, Entity, IntoElement, Pixels, Render, TestAppContext,
    VisualTestContext, Window, point, px, size,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_ui::swiftui::motion::ease_in_out;

use super::rise_manual_list_types::{
    RiseManualListItem, RiseManualListItemMountPolicy, RiseManualListItemProvider,
    RiseManualListKeyboardPresentationPolicy, RiseManualListScrollMetrics, RiseManualListTransaction,
    RiseManualListTransactionAnchor, RiseManualListTransactionEdgeMutation, RiseManualListViewportAnchor,
    RiseManualListViewportAnchorAnimationPolicy, RiseManualListViewportResizeAnimation,
};
use super::rise_manual_list_view::RiseManualListView;
use crate::modules::chat::components::chat::async_render::rise_message_list_view::{
    RiseMessageKeyboardRestingInsetPolicy, RiseMessageKeyboardViewportPolicy,
};

struct TestRow;

impl Render for TestRow {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

struct ListHarness<'a> {
    list: Entity<RiseManualListView>,
    width: f32,
    cx: &'a mut VisualTestContext,
}

impl<'a> ListHarness<'a> {
    fn new(cx: &'a mut TestAppContext, width: f32, height: f32) -> Self {
        let (list, cx) = cx.add_window_view(|window, cx| RiseManualListView::new(window, cx));
        cx.simulate_resize(size(px(width), px(height)));
        let mut harness = Self { list, width, cx };
        harness.resize(height);
        harness
    }

    fn update<R>(
        &mut self,
        f: impl FnOnce(&mut RiseManualListView, &mut Window, &mut Context<RiseManualListView>) -> R,
    ) -> R {
        let list = self.list.clone();
        self.cx.update(|window, app| list.update(app, |list, cx| f(list, window, cx)))
    }

    fn resize(&mut self, height: f32) {
        let frame = Bounds::new(point(px(0.0), px(0.0)), size(px(self.width), px(height)));
        self.update(|list, window, cx| list.layout_subviews(frame, window, cx));
    }

    fn metrics(&mut self) -> RiseManualListScrollMetrics {
        self.update(|list, _, _| list.scroll_metrics())
    }

    fn enqueue(&mut self, transaction: RiseManualListTransaction) {
        self.update(|list, window, cx| list.enqueue(transaction, None, None, window, cx));
    }

    fn set_insets(&mut self, top: f32, bottom: f32) {
        self.update(|list, window, cx| list.set_viewport_insets(insets(top, bottom), window, cx));
    }

    fn set_offset(&mut self, value: Pixels) -> bool {
        self.update(|list, window, cx| list.set_content_offset_y(value, false, true, window, cx))
    }

    fn set_scrolling(&mut self, is_scrolling: Option<bool>) {
        self.update(|list, window, cx| list.set_user_scrolling_override_for_testing(is_scrolling, window, cx));
    }

    fn anchor(&mut self, id: &str) -> RiseManualListViewportAnchor {
        self.update(|list, _, _| list.capture_viewport_anchor(id)).expect("viewport anchor")
    }

    fn prepare_resize(&mut self, duration: f64) {
        let animation = RiseManualListViewportResizeAnimation::new(
            duration,
            ease_in_out(duration),
            RiseClock::monotonic().as_secs_f64(),
        );
        self.update(|list, window, cx| list.prepare_viewport_resize_animation(animation, window, cx));
    }

    fn keyboard_content_translation(&mut self) -> Pixels {
        self.update(|list, _, _| list.keyboard_content_translation_for_testing())
    }

    fn keyboard_sticky_translation(&mut self) -> Pixels {
        self.update(|list, _, _| list.keyboard_sticky_translation_for_testing())
    }

    fn last_anchor_animation_offset(&mut self) -> Option<Pixels> {
        self.update(|list, _, _| list.last_viewport_anchor_animation_offset_for_testing())
    }

    fn last_clip_animation_start_height(&mut self) -> Option<Pixels> {
        self.update(|list, _, _| list.last_viewport_clip_animation_start_height_for_testing())
    }
}

fn insets(top: f32, bottom: f32) -> Edges<Pixels> {
    Edges { top: px(top), right: px(0.0), bottom: px(bottom), left: px(0.0) }
}

fn close(value: f32, expected: f32) -> bool {
    (value - expected).abs() <= 0.5
}

fn row_ids(count: usize) -> Vec<String> {
    (0..count).map(|index| format!("row-{index}")).collect()
}

fn ids(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

fn item(height: f32, reuse_identifier: &str, content_version: i64) -> RiseManualListItem {
    RiseManualListItem::new(
        height,
        reuse_identifier.to_string(),
        content_version,
        RiseManualListItemMountPolicy::Immediate,
        None,
        None,
        None,
        None,
        Rc::new(|reusable: Option<AnyView>, _: &mut Window, cx: &mut App| {
            reusable.unwrap_or_else(|| cx.new(|_| TestRow).into())
        }),
    )
}

fn fixed_provider(height: f32, reuse_identifier: &'static str) -> RiseManualListItemProvider {
    Rc::new(move |_: &str, _: f32, _: &mut Window, _: &mut App| item(height, reuse_identifier, 1))
}

fn transaction(
    ids: Vec<String>,
    anchor: RiseManualListTransactionAnchor,
    provider: RiseManualListItemProvider,
) -> RiseManualListTransaction {
    RiseManualListTransaction::new(ids, anchor, None, None, None, None, None, false, HashSet::new(), provider)
}

fn edge_transaction(
    ids: Vec<String>,
    anchor: RiseManualListTransactionAnchor,
    updated_id: &str,
    edge_mutation: RiseManualListTransactionEdgeMutation,
    provider: RiseManualListItemProvider,
) -> RiseManualListTransaction {
    RiseManualListTransaction::new(
        ids,
        anchor,
        None,
        Some(HashSet::from([updated_id.to_string()])),
        None,
        None,
        Some(edge_mutation),
        true,
        HashSet::new(),
        provider,
    )
}

#[gpui::test]
fn latest_history_clamps_only_bottom_overscroll(cx: &mut TestAppContext) {
    let mut list = ListHarness::new(cx, 320.0, 180.0);
    list.set_insets(10.0, 70.0);
    list.resize(180.0);
    list.enqueue(transaction(
        row_ids(8),
        RiseManualListTransactionAnchor::Bottom,
        fixed_provider(60.0, "bottom-overscroll"),
    ));

    let bottom_offset = list.metrics().maximum_content_offset_y;
    assert!(list.update(|list, window, cx| {
        list.set_content_offset_y(bottom_offset + px(48.0), false, false, window, cx)
    }));
    assert!(list.metrics().content_offset_y > bottom_offset);

    list.update(|list, _, cx| list.set_prevents_bottom_overscroll(true, cx));
    assert!(close(f32::from(list.metrics().content_offset_y), f32::from(bottom_offset)));

    assert!(list.set_offset(bottom_offset - px(80.0)));
    assert!(close(f32::from(list.metrics().content_offset_y), f32::from(bottom_offset) - 80.0));
}

#[gpui::test]
fn active_scroll_height_correction_keeps_the_visible_anchor_stable(cx: &mut TestAppContext) {
    let mut list = ListHarness::new(cx, 320.0, 150.0);
    list.update(|list, window, cx| list.set_inter_item_spacing(px(4.0), window, cx));
    list.resize(150.0);

    let row_ids = row_ids(8);
    let heights: Rc<RefCell<HashMap<String, f32>>> =
        Rc::new(RefCell::new(row_ids.iter().map(|id| (id.clone(), 72.0)).collect()));
    let provider: RiseManualListItemProvider = {
        let heights = heights.clone();
        Rc::new(move |id: &str, _: f32, _: &mut Window, _: &mut App| {
            item(heights.borrow().get(id).copied().unwrap_or(44.0), "active-height-correction", 1)
        })
    };
    list.enqueue(transaction(row_ids.clone(), RiseManualListTransactionAnchor::Top, provider.clone()));
    assert!(list.set_offset(px(152.0)));
    list.set_scrolling(Some(true));
    let visible_id =
        list.update(|list, _, _| list.visible_ids().first().cloned()).expect("visible row");
    let before = list.anchor(&visible_id);
    let previous_content_height = f32::from(list.metrics().content_height);

    heights.borrow_mut().insert(visible_id.clone(), 44.0);
    list.enqueue(edge_transaction(
        row_ids,
        RiseManualListTransactionAnchor::Item {
            id: visible_id.clone(),
            viewport_offset: before.viewport_offset,
        },
        &visible_id,
        RiseManualListTransactionEdgeMutation::default(),
        provider,
    ));
    list.cx.run_until_parked();

    let after = list.anchor(&visible_id);
    assert!(close(after.viewport_offset, before.viewport_offset));
    assert!(close(f32::from(list.metrics().content_height), previous_content_height - 28.0));
    list.set_scrolling(Some(false));
}

#[gpui::test]
fn retained_boundary_height_can_change_without_moving_its_viewport_anchor(cx: &mut TestAppContext) {
    let mut list = ListHarness::new(cx, 320.0, 120.0);
    list.update(|list, window, cx| list.set_inter_item_spacing(px(4.0), window, cx));
    list.resize(120.0);

    let heights: Rc<RefCell<HashMap<String, f32>>> = Rc::new(RefCell::new(HashMap::from([
        ("a".to_string(), 60.0),
        ("b".to_string(), 60.0),
        ("c".to_string(), 60.0),
    ])));
    let provider: RiseManualListItemProvider = {
        let heights = heights.clone();
        Rc::new(move |id: &str, _: f32, _: &mut Window, _: &mut App| {
            let height = heights.borrow().get(id).copied();
            item(height.unwrap_or(44.0), "test", height.unwrap_or(0.0) as i64)
        })
    };

    list.enqueue(transaction(ids(&["a", "b", "c"]), RiseManualListTransactionAnchor::Top, provider.clone()));
    let before = list.anchor("a");

    *heights.borrow_mut() = HashMap::from([
        ("x".to_string(), 40.0),
        ("y".to_string(), 50.0),
        ("a".to_string(), 30.0),
        ("b".to_string(), 60.0),
        ("c".to_string(), 60.0),
    ]);
    list.enqueue(edge_transaction(
        ids(&["x", "y", "a", "b", "c"]),
        RiseManualListTransactionAnchor::PreserveVisible,
        "a",
        RiseManualListTransactionEdgeMutation::new(0, 0, 2, 0),
        provider,
    ));

    let after = list.anchor("a");
    assert!(close(after.viewport_offset, before.viewport_offset));
    let target = RiseManualListViewportAnchor::new("x".to_string(), 0.0);
    assert!(list.update(|list, window, cx| list.scroll_to(&target, false, window, cx)));
    let x = list.anchor("x");
    let y = list.anchor("y");
    let repositioned_a = list.anchor("a");
    assert!(close(y.viewport_offset - x.viewport_offset, 44.0));
    assert!(close(repositioned_a.viewport_offset - y.viewport_offset, 54.0));
    assert!(close(f32::from(list.metrics().content_height), 256.0));
}

#[gpui::test]
fn viewport_inset_change_pins_only_an_exact_bottom_anchor(cx: &mut TestAppContext) {
    let mut list = ListHarness::new(cx, 320.0, 180.0);
    list.set_insets(10.0, 20.0);
    list.resize(180.0);
    list.enqueue(transaction(row_ids(10), RiseManualListTransactionAnchor::Bottom, fixed_provider(60.0, "test")));

    assert!(list.metrics().distance_from_bottom <= px(0.5));
    list.set_insets(10.0, 100.0);
    assert!(list.metrics().distance_from_bottom <= px(0.5));

    let reading_offset = list.metrics().maximum_content_offset_y - px(50.0);
    assert!(list.set_offset(reading_offset));
    let before_offset = f32::from(list.metrics().content_offset_y);
    let before_anchor = list.update(|list, _, _| list.capture_visible_anchor()).expect("visible anchor");

    list.set_insets(10.0, 150.0);

    let after_anchor = list.anchor(&before_anchor.id);
    assert!(close(f32::from(list.metrics().content_offset_y), before_offset));
    assert!(close(after_anchor.viewport_offset, before_anchor.viewport_offset));
    assert!(list.metrics().distance_from_bottom >= px(99.5));
}

#[gpui::test]
fn progressive_keyboard_insets_move_the_bottom_anchor_by_each_frame_delta(cx: &mut TestAppContext) {
    let mut list = ListHarness::new(cx, 320.0, 500.0);
    list.set_insets(10.0, 70.0);
    list.resize(500.0);
    list.enqueue(transaction(row_ids(10), RiseManualListTransactionAnchor::Bottom, fixed_provider(60.0, "test")));

    let mut previous_inset = 70.0_f32;
    let mut previous_anchor = list.anchor("row-9");
    for next_inset in [100.0_f32, 160.0, 260.0, 330.0] {
        list.set_insets(10.0, next_inset);
        let next_anchor = list.anchor("row-9");
        let expected_movement = -(next_inset - previous_inset);
        assert!(close(next_anchor.viewport_offset - previous_anchor.viewport_offset, expected_movement));
        previous_inset = next_inset;
        previous_anchor = next_anchor;
    }
}

#[gpui::test]
fn native_viewport_resize_during_gesture_preserves_the_finger_anchor(cx: &mut TestAppContext) {
    let mut list = ListHarness::new(cx, 320.0, 500.0);
    list.set_insets(10.0, 70.0);
    list.resize(500.0);
    list.enqueue(transaction(row_ids(12), RiseManualListTransactionAnchor::Bottom, fixed_provider(60.0, "test")));

    assert!(list.metrics().distance_from_bottom <= px(0.5));
    let before = list.anchor("row-11");

    list.set_scrolling(Some(true));
    list.prepare_resize(0.25);
    list.resize(400.0);

    let during = list.anchor("row-11");
    assert!(close(during.viewport_offset, before.viewport_offset));
    assert!(list.metrics().distance_from_bottom >= px(99.5));
    assert_eq!(list.last_anchor_animation_offset(), None);
    assert_eq!(list.last_clip_animation_start_height(), None);

    list.set_insets(10.0, 34.0);
    let after_inset_change = list.anchor("row-11");
    assert!(close(after_inset_change.viewport_offset, before.viewport_offset));

    list.set_scrolling(Some(false));
    let after = list.anchor("row-11");
    assert!(close(after.viewport_offset, before.viewport_offset));
    assert!(list.metrics().distance_from_bottom >= px(63.5));
}

#[gpui::test]
fn native_viewport_resize_keeps_bottom_pinned_without_an_active_gesture(cx: &mut TestAppContext) {
    let mut list = ListHarness::new(cx, 320.0, 500.0);
    list.set_insets(10.0, 70.0);
    list.resize(500.0);
    list.enqueue(transaction(row_ids(12), RiseManualListTransactionAnchor::Bottom, fixed_provider(60.0, "test")));

    assert!(list.metrics().distance_from_bottom <= px(0.5));
    list.resize(400.0);
    assert!(list.metrics().distance_from_bottom <= px(0.5));
}

#[test]
fn viewport_anchor_flip_starts_at_the_previous_visual_position() {
    assert_eq!(RiseManualListViewportAnchorAnimationPolicy::inverse_translation_y(-290.0, -390.0, 0.0), 100.0);
    assert_eq!(RiseManualListViewportAnchorAnimationPolicy::inverse_translation_y(-390.0, -290.0, 0.0), -100.0);
    assert_eq!(RiseManualListViewportAnchorAnimationPolicy::inverse_translation_y(-390.0, -440.0, 35.0), 85.0);
}

#[test]
fn keyboard_presentation_frames_resolve_symmetrically_without_endpoint_replay() {
    let overlaps = [0.0_f32, 12.0, 29.0, 80.0, 180.0, 325.0];
    let resolve = |overlap: f32| RiseMessageKeyboardViewportPolicy::resolved_inset(72.0, 101.0, overlap, true, None);
    let opening: Vec<f32> = overlaps.iter().copied().map(resolve).collect();
    let mut closing: Vec<f32> = overlaps.iter().rev().copied().map(resolve).collect();
    closing.reverse();

    assert_eq!(opening, closing);
    assert_eq!(opening, vec![101.0, 101.0, 101.0, 152.0, 252.0, 397.0]);
    assert_eq!(RiseMessageKeyboardViewportPolicy::resolved_inset(398.0, 101.0, 325.0, false, None), 398.0);
}

#[test]
fn visible_keyboard_does_not_replay_the_resting_safe_area_inset() {
    let full_height = 844.0_f32;
    let keyboard_height = 336.0_f32;
    let manual_surface_inset = 398.0_f32;
    let configured_keyboard_inset = 62.0_f32;
    let keyboard_resting_inset = 91.0_f32;
    let final_inset = RiseMessageKeyboardRestingInsetPolicy::resolved_inset(
        configured_keyboard_inset,
        keyboard_resting_inset,
        0.0,
        true,
    );

    assert_eq!(full_height - manual_surface_inset, full_height - keyboard_height - final_inset);
    assert_eq!(
        RiseMessageKeyboardRestingInsetPolicy::resolved_inset(
            configured_keyboard_inset,
            keyboard_resting_inset,
            0.0,
            false,
        ),
        keyboard_resting_inset
    );
    assert_eq!(
        RiseMessageKeyboardRestingInsetPolicy::resolved_inset(
            configured_keyboard_inset,
            keyboard_resting_inset,
            keyboard_height,
            true,
        ),
        keyboard_resting_inset
    );
}

#[test]
fn keyboard_presentation_uses_symmetric_compositor_translations() {
    assert_eq!(
        RiseManualListKeyboardPresentationPolicy::content_translation_y(px(70.0), px(330.0), true),
        px(-260.0)
    );
    assert_eq!(
        RiseManualListKeyboardPresentationPolicy::content_translation_y(px(330.0), px(70.0), true),
        px(260.0)
    );
    assert_eq!(
        RiseManualListKeyboardPresentationPolicy::content_translation_y(px(70.0), px(330.0), false),
        Pixels::ZERO
    );
    assert_eq!(
        RiseManualListKeyboardPresentationPolicy::sticky_translation_y(Some(px(60.0)), Some(px(320.0))),
        px(-260.0)
    );
}

#[gpui::test]
fn keyboard_presentation_commits_once_and_always_returns_to_resting_inset(cx: &mut TestAppContext) {
    let mut list = ListHarness::new(cx, 320.0, 500.0);
    list.set_insets(10.0, 70.0);
    list.update(|list, window, cx| list.set_bottom_sticky_viewport_inset(Some(px(60.0)), window, cx));
    list.resize(500.0);
    list.enqueue(transaction(row_ids(12), RiseManualListTransactionAnchor::Bottom, fixed_provider(60.0, "test")));

    let opened_insets = insets(10.0, 330.0);
    list.update(|list, window, cx| {
        list.animate_keyboard_viewport_transition(opened_insets, Some(px(320.0)), 0.0, window, cx)
    });
    assert!(list.update(|list, _, _| list.has_active_keyboard_viewport_transition()));
    assert_eq!(list.update(|list, _, _| list.viewport_insets().bottom), px(70.0));
    assert_eq!(list.keyboard_content_translation(), px(-260.0));
    assert_eq!(list.keyboard_sticky_translation(), px(-260.0));

    list.update(|list, window, cx| {
        list.finish_keyboard_viewport_transition(opened_insets, Some(px(320.0)), window, cx)
    });
    assert!(!list.update(|list, _, _| list.has_active_keyboard_viewport_transition()));
    assert_eq!(list.update(|list, _, _| list.viewport_insets().bottom), px(330.0));
    assert!(list.metrics().distance_from_bottom <= px(0.5));
    assert_eq!(list.keyboard_content_translation(), Pixels::ZERO);

    let resting_insets = insets(10.0, 70.0);
    list.update(|list, window, cx| {
        list.animate_keyboard_viewport_transition(resting_insets, Some(px(60.0)), 0.0, window, cx)
    });
    assert_eq!(list.update(|list, _, _| list.viewport_insets().bottom), px(330.0));
    assert_eq!(list.keyboard_content_translation(), px(260.0));

    list.update(|list, window, cx| {
        list.finish_keyboard_viewport_transition(resting_insets, Some(px(60.0)), window, cx)
    });
    assert!(!list.update(|list, _, _| list.has_active_keyboard_viewport_transition()));
    assert_eq!(list.update(|list, _, _| list.viewport_insets().bottom), px(70.0));
    assert_eq!(list.update(|list, _, _| list.bottom_sticky_viewport_inset()), Some(px(60.0)));
    assert!(list.metrics().distance_from_bottom <= px(0.5));
    assert_eq!(list.keyboard_content_translation(), Pixels::ZERO);
}

#[gpui::test]
fn pre_scrolled_keyboard_dismissal_preserves_the_users_scroll_distance(cx: &mut TestAppContext) {
    let mut list = ListHarness::new(cx, 320.0, 500.0);
    list.set_insets(10.0, 330.0);
    list.update(|list, window, cx| list.set_bottom_sticky_viewport_inset(Some(px(320.0)), window, cx));
    list.resize(500.0);
    list.enqueue(transaction(row_ids(16), RiseManualListTransactionAnchor::Bottom, fixed_provider(60.0, "test")));

    let reading_offset = list.metrics().maximum_content_offset_y - px(40.0);
    assert!(list.set_offset(reading_offset));
    let resting_insets = insets(10.0, 70.0);
    list.update(|list, window, cx| {
        list.animate_keyboard_viewport_transition(resting_insets, Some(px(60.0)), 0.0, window, cx)
    });
    assert_eq!(list.keyboard_content_translation(), px(260.0));

    list.set_scrolling(Some(true));
    list.update(|list, window, cx| {
        list.finish_keyboard_viewport_transition(resting_insets, Some(px(60.0)), window, cx)
    });

    assert_eq!(list.update(|list, _, _| list.viewport_insets().bottom), px(70.0));
    assert!(close(f32::from(list.metrics().distance_from_bottom), 40.0));
    assert_eq!(list.keyboard_content_translation(), Pixels::ZERO);
    list.set_scrolling(None);
}

#[test]
fn stale_viewport_resize_timing_cannot_animate_an_unrelated_resize() {
    let transition = RiseManualListViewportResizeAnimation::new(0.25, ease_in_out(0.25), 10.0);

    assert!((transition.remaining_duration(10.1).unwrap_or(0.0) - 0.15).abs() < 0.001);
    assert_eq!(transition.remaining_duration(10.3), None);
}

#[gpui::test]
fn native_keyboard_resize_uses_one_visual_flip_while_model_stays_bottom_pinned(cx: &mut TestAppContext) {
    let mut list = ListHarness::new(cx, 320.0, 500.0);
    list.set_insets(10.0, 70.0);
    list.resize(500.0);
    list.enqueue(transaction(row_ids(12), RiseManualListTransactionAnchor::Bottom, fixed_provider(60.0, "test")));

    list.prepare_resize(1.0);
    list.resize(400.0);

    assert!(list.metrics().distance_from_bottom <= px(0.5));
    assert!(close(f32::from(list.last_anchor_animation_offset().unwrap_or(Pixels::ZERO)), 100.0));
    assert!(close(f32::from(list.last_clip_animation_start_height().unwrap_or(Pixels::ZERO)), 500.0));
}

#[gpui::test]
fn keyboard_hide_animates_the_list_and_clip_back_down(cx: &mut TestAppContext) {
    let mut list = ListHarness::new(cx, 320.0, 400.0);
    list.set_insets(10.0, 34.0);
    list.resize(400.0);
    list.enqueue(transaction(row_ids(12), RiseManualListTransactionAnchor::Bottom, fixed_provider(60.0, "test")));

    list.prepare_resize(1.0);
    list.resize(500.0);

    assert!(list.metrics().distance_from_bottom <= px(0.5));
    assert!(close(f32::from(list.last_anchor_animation_offset().unwrap_or(Pixels::ZERO)), -100.0));
    assert!(close(f32::from(list.last_clip_animation_start_height().unwrap_or(Pixels::ZERO)), 400.0));
}

#[gpui::test]
fn keyboard_endpoint_keeps_the_old_bottom_media_band_mounted(cx: &mut TestAppContext) {
    let mut list = ListHarness::new(cx, 320.0, 500.0);
    list.update(|list, window, cx| list.set_preload_distance(px(0.0), window, cx));
    list.set_insets(10.0, 70.0);
    list.resize(500.0);
    list.enqueue(transaction(row_ids(40), RiseManualListTransactionAnchor::Top, fixed_provider(60.0, "media")));
    assert!(list.set_offset(px(600.0)));
    let old_bottom_id = list.update(|list, _, _| list.visible_ids().last().cloned()).expect("visible row");
    assert!(list.update(|list, _, _| list.is_item_mounted_for_testing(&old_bottom_id)));

    list.prepare_resize(1.0);
    list.resize(100.0);

    assert!(!list.update(|list, _, _| list.visible_ids().contains(&old_bottom_id)));
    assert!(list.update(|list, _, _| list.is_item_mounted_for_testing(&old_bottom_id)));
    assert!(close(f32::from(list.last_clip_animation_start_height().unwrap_or(Pixels::ZERO)), 500.0));
}
