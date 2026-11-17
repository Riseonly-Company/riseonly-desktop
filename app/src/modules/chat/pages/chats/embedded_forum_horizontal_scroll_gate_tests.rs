use std::rc::Rc;

use gpui::{App, Modifiers, ScrollDelta, ScrollHandle, ScrollWheelEvent, TouchPhase, Window, point, px};
use rise_ui::guide_anchor_ui::guide_scroll_host::GuideScrollHostMount;

use super::embedded_forum_horizontal_scroll_gate::{
    EmbeddedForumHorizontalPanCallbacks, EmbeddedForumHorizontalScrollGateCoordinator, PanDelivery, PanSource,
};
use crate::modules::chat::pages::chats::embedded_forum_transition::EmbeddedForumTransitionMetrics;

const PAST_ACTIVATION: f32 = 20.0;

fn noop_callbacks() -> EmbeddedForumHorizontalPanCallbacks {
    EmbeddedForumHorizontalPanCallbacks {
        on_changed: Rc::new(|_: f32, _: &mut Window, _: &mut App| {}),
        on_ended: Rc::new(|_: f32, _: f32, _: &mut Window, _: &mut App| {}),
        on_cancelled: Rc::new(|_: &mut Window, _: &mut App| {}),
    }
}

fn wheel(x: f32, touch_phase: TouchPhase) -> ScrollWheelEvent {
    ScrollWheelEvent {
        position: point(px(0.0), px(0.0)),
        delta: ScrollDelta::Pixels(point(px(x), px(0.0))),
        modifiers: Modifiers::default(),
        touch_phase,
    }
}

#[test]
fn wheel_ended_after_won_pan_reaches_the_list_and_delivers_ended() {
    let mount = GuideScrollHostMount::new(ScrollHandle::new());
    let scroll_view = mount.host();
    let mut coordinator = EmbeddedForumHorizontalScrollGateCoordinator::new(Some(noop_callbacks()), true);
    assert!(coordinator.attach(&scroll_view).is_none());

    let started = coordinator.handle_scroll_wheel(&wheel(0.0, TouchPhase::Started), true);
    assert!(!started.consumed);

    let distance = EmbeddedForumTransitionMetrics::SWIPE_MINIMUM_DISTANCE + PAST_ACTIVATION;
    let moved = coordinator.handle_scroll_wheel(&wheel(distance, TouchPhase::Moved), true);
    assert!(moved.consumed);
    assert!(matches!(moved.delivery, PanDelivery::Changed(_)));

    let ended = coordinator.handle_scroll_wheel(&wheel(0.0, TouchPhase::Ended), true);
    assert!(!ended.consumed);
    assert!(matches!(
        ended.delivery,
        PanDelivery::Ended(translation, _) if (translation - PAST_ACTIVATION).abs() < 0.001
    ));
}

#[test]
fn mouse_release_after_won_pan_stays_consumed_so_the_row_gets_no_click() {
    let mount = GuideScrollHostMount::new(ScrollHandle::new());
    let scroll_view = mount.host();
    let mut coordinator = EmbeddedForumHorizontalScrollGateCoordinator::new(Some(noop_callbacks()), true);
    assert!(coordinator.attach(&scroll_view).is_none());

    coordinator.touches_began(point(px(0.0), px(0.0)), PanSource::Mouse);
    let distance = EmbeddedForumTransitionMetrics::SWIPE_MINIMUM_DISTANCE + PAST_ACTIVATION;
    assert!(coordinator.mouse_moved(point(px(distance), px(0.0))).consumed);

    let ended = coordinator.touches_ended(PanSource::Mouse);
    assert!(ended.consumed);
    assert!(matches!(ended.delivery, PanDelivery::Ended(_, _)));
}

#[test]
fn gate_enablement_and_reparenting_keep_one_configured_dismiss_pan() {
    let callbacks = EmbeddedForumHorizontalPanCallbacks {
        on_changed: Rc::new(|_: f32, _: &mut Window, _: &mut App| {}),
        on_ended: Rc::new(|_: f32, _: f32, _: &mut Window, _: &mut App| {}),
        on_cancelled: Rc::new(|_: &mut Window, _: &mut App| {}),
    };
    let mut coordinator = EmbeddedForumHorizontalScrollGateCoordinator::new(Some(callbacks.clone()), true);
    let first_mount = GuideScrollHostMount::new(ScrollHandle::new());
    let second_mount = GuideScrollHostMount::new(ScrollHandle::new());
    let first_scroll_view = first_mount.host();
    let second_scroll_view = second_mount.host();

    assert!(coordinator.attach(&first_scroll_view).is_none());
    assert!(coordinator.horizontal_pan().is_enabled());
    assert!(coordinator.horizontal_pan().cancels_touches_in_view());

    assert!(coordinator.update(Some(callbacks.clone()), false).is_none());
    assert!(!coordinator.horizontal_pan().is_enabled());

    assert!(coordinator.update(Some(callbacks), true).is_none());
    assert!(coordinator.horizontal_pan().is_enabled());

    assert!(coordinator.attach(&second_scroll_view).is_none());
    let view = coordinator.horizontal_pan().view();
    assert!(view.as_ref().is_some_and(|view| view.same_host(&second_scroll_view)));
    assert!(!view.as_ref().is_some_and(|view| view.same_host(&first_scroll_view)));
    assert!(coordinator.horizontal_pan().is_enabled());

    assert!(coordinator.detach().is_none());
    assert!(coordinator.horizontal_pan().view().is_none());
    assert!(!coordinator.horizontal_pan().is_enabled());
}
