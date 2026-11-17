use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{AppContext, Entity, TestAppContext};

use super::embedded_forum_transition::{
    EmbeddedForumPresentationSessionPolicy, EmbeddedForumRailMotionDriver, EmbeddedForumRailMotionTarget,
    EmbeddedForumRailSnapshot, EmbeddedForumTransitionMetrics, EmbeddedForumTransitionState,
};

const CONTAINER_WIDTH: f32 = 390.0;

#[derive(Default)]
struct ForumRailMotionTestTarget {
    snapshots: RefCell<Vec<EmbeddedForumRailSnapshot>>,
}

impl EmbeddedForumRailMotionTarget for ForumRailMotionTestTarget {
    fn apply_rail_motion(&self, snapshot: &EmbeddedForumRailSnapshot) {
        self.snapshots.borrow_mut().push(*snapshot);
    }
}

impl ForumRailMotionTestTarget {
    fn last_progress(&self) -> Option<f32> {
        self.snapshots.borrow().last().map(|snapshot| snapshot.progress(CONTAINER_WIDTH))
    }
}

fn new_transition(cx: &mut TestAppContext) -> Entity<EmbeddedForumTransitionState> {
    cx.new(|_| EmbeddedForumTransitionState::new())
}

fn presented(cx: &mut TestAppContext) -> Entity<EmbeddedForumTransitionState> {
    let transition = new_transition(cx);
    transition.update(cx, |transition, cx| {
        transition.prepare_for_presentation(cx);
        transition.set_presented(true, cx);
        transition.release_structural_updates_if_idle(cx);
    });
    transition
}

#[gpui::test]
fn native_rail_reads_do_not_subscribe_hosting_graphs_to_motion(cx: &mut TestAppContext) {
    let transition = new_transition(cx);
    let driver = transition.read_with(cx, |transition, _| transition.rail_motion().clone());
    assert_eq!(driver.snapshot().progress(CONTAINER_WIDTH), 0.0);
    transition.update(cx, |transition, cx| {
        transition.prepare_for_presentation(cx);
        transition.set_presented(true, cx);
        transition.begin_interactive_gesture(cx);
        transition.update_drag_offset(120.0, CONTAINER_WIDTH, cx);
    });
    assert_eq!(driver.snapshot().drag_offset, 120.0);
    transition.update(cx, |transition, cx| transition.set_presented(false, cx));
    assert_eq!(driver.snapshot().progress(CONTAINER_WIDTH), 0.0);
}

#[gpui::test]
fn native_rail_follows_open_pan_snap_back_and_immediate_dismiss(cx: &mut TestAppContext) {
    let transition = new_transition(cx);
    let target = Rc::new(ForumRailMotionTestTarget::default());
    let attached: Rc<dyn EmbeddedForumRailMotionTarget> = target.clone();
    transition.read_with(cx, |transition, _| transition.rail_motion().attach(attached.clone()));
    assert_eq!(target.last_progress(), Some(0.0));
    transition.update(cx, |transition, cx| transition.set_presented(true, cx));
    assert_eq!(target.last_progress(), Some(1.0));
    transition.update(cx, |transition, cx| {
        transition.begin_interactive_gesture(cx);
        transition.update_drag_offset(80.0, CONTAINER_WIDTH, cx);
    });
    assert_eq!(target.last_progress(), Some(0.75));
    transition.update(cx, |transition, cx| transition.snap_back_after_interaction(cx));
    assert_eq!(target.last_progress(), Some(1.0));
    transition.update(cx, |transition, cx| transition.dismiss_immediately(cx));
    assert_eq!(target.last_progress(), Some(0.0));
}

#[test]
fn native_rail_does_not_retain_recycled_surfaces() {
    let driver = EmbeddedForumRailMotionDriver::new();
    let target: Rc<dyn EmbeddedForumRailMotionTarget> = Rc::new(ForumRailMotionTestTarget::default());
    let weak_target = Rc::downgrade(&target);
    driver.attach(target);
    assert!(weak_target.upgrade().is_none());
    driver.update(EmbeddedForumRailSnapshot { is_presented: true, drag_offset: 0.0 }, false);
}

#[test]
fn detached_rail_surface_stops_receiving_motion_and_reattaches_at_current_state() {
    let driver = EmbeddedForumRailMotionDriver::new();
    let target = Rc::new(ForumRailMotionTestTarget::default());
    let attached: Rc<dyn EmbeddedForumRailMotionTarget> = target.clone();
    driver.attach(attached.clone());
    driver.detach(&attached);
    driver.update(EmbeddedForumRailSnapshot { is_presented: true, drag_offset: 0.0 }, false);
    assert_eq!(target.snapshots.borrow().len(), 1);
    driver.attach(attached);
    assert_eq!(target.last_progress(), Some(1.0));
}

#[gpui::test]
fn cancelled_pan_remains_busy_until_visual_snap_back_finishes(cx: &mut TestAppContext) {
    let transition = presented(cx);
    let settled_generation = transition.read_with(cx, |transition, _| {
        assert!(!transition.structural_updates_blocked());
        transition.structural_refresh_generation()
    });
    transition.update(cx, |transition, cx| {
        transition.begin_interactive_gesture(cx);
        transition.update_drag_offset(90.0, CONTAINER_WIDTH, cx);
    });
    transition.read_with(cx, |transition, _| {
        assert!(transition.structural_updates_blocked());
        assert_eq!(transition.structural_refresh_generation(), settled_generation);
    });

    transition.update(cx, |transition, cx| transition.snap_back_after_interaction(cx));
    transition.read_with(cx, |transition, _| {
        assert!(!transition.is_interactive_gesture_active());
        assert!(transition.is_interaction_settling());
        assert!(!transition.has_interactive_offset());
    });

    cx.executor()
        .advance_clock(Duration::from_secs_f64(EmbeddedForumTransitionMetrics::ANIMATION_DURATION + 0.04));
    cx.run_until_parked();
    transition.read_with(cx, |transition, _| {
        assert!(!transition.is_interaction_settling());
        assert!(!transition.structural_updates_blocked());
        assert_eq!(transition.structural_refresh_generation(), settled_generation + 1);
    });
}

#[gpui::test]
fn accepted_close_keeps_topic_hierarchy_frozen_until_removal(cx: &mut TestAppContext) {
    let transition = presented(cx);
    let settled_generation = transition.read_with(cx, |transition, _| transition.structural_refresh_generation());

    transition.update(cx, |transition, cx| {
        transition.begin_interactive_gesture(cx);
        transition.update_drag_offset(120.0, CONTAINER_WIDTH, cx);
        transition.end_interactive_gesture(cx);
        transition.set_presented(false, cx);
    });

    transition.read_with(cx, |transition, _| {
        assert!(transition.structural_updates_blocked());
        assert_eq!(transition.structural_refresh_generation(), settled_generation);
    });
}

#[test]
fn stale_close_cannot_unmount_a_reopened_copy_of_the_same_chat() {
    assert!(!EmbeddedForumPresentationSessionPolicy::should_cleanup(Some("forum-a"), 12, false, "forum-a", 11));
}

#[test]
fn current_close_cleans_up_only_after_presentation_ends() {
    assert!(!EmbeddedForumPresentationSessionPolicy::should_cleanup(Some("forum-a"), 12, true, "forum-a", 12));
    assert!(EmbeddedForumPresentationSessionPolicy::should_cleanup(Some("forum-a"), 12, false, "forum-a", 12));
}

#[test]
fn cleanup_never_crosses_between_forum_chats() {
    assert!(!EmbeddedForumPresentationSessionPolicy::should_cleanup(Some("forum-b"), 12, false, "forum-a", 12));
}

#[test]
fn closing_from_a_long_drag_never_pulls_the_panel_back() {
    let driver = EmbeddedForumRailMotionDriver::new();
    let distance = EmbeddedForumTransitionMetrics::travel_distance(CONTAINER_WIDTH);
    let drag_offset = distance * 0.66;
    driver.update(EmbeddedForumRailSnapshot { is_presented: true, drag_offset }, false);
    driver.update(EmbeddedForumRailSnapshot { is_presented: false, drag_offset: 0.0 }, true);

    let start = rise_core::foundation::foundation_date::RiseClock::monotonic();
    let mut previous = driver.panel_offset_at(start, CONTAINER_WIDTH);
    assert!(previous >= drag_offset - 1.0, "starts where the drag left it, got {previous}");
    for frame in 1..=120_u32 {
        let offset = driver.panel_offset_at(start + Duration::from_millis(u64::from(frame) * 16), CONTAINER_WIDTH);
        assert!(offset >= previous - 0.001, "frame {frame}: {offset} went back from {previous}");
        previous = offset;
    }
    assert!((previous - distance).abs() < 0.5, "ends off screen, got {previous}");
}
