use std::collections::HashSet;

use gpui::{App, ElementId, EntityId, Global, Pixels, Point, ScrollDelta, ScrollWheelEvent, TouchPhase, px};

/// splits vertical list scroll from horizontal row gestures like the reply swipe
/// by the dominant direction of the gesture first motion
/// gate stops nothing, handlers ask allows_pan first
pub struct RiseVerticalScrollGestureGate {
    owner_view: EntityId,
    prioritized_pans: HashSet<ElementId>,
    vertical_intent: Option<bool>,
}

impl RiseVerticalScrollGestureGate {
    pub fn new(owner_view: EntityId) -> Self {
        Self {
            owner_view,
            prioritized_pans: HashSet::new(),
            vertical_intent: None,
        }
    }

    pub fn owner_view(&self) -> EntityId {
        self.owner_view
    }

    pub fn require_vertical_intent_to_fail(&mut self, before_gesture_recognizer: ElementId) {
        self.prioritized_pans.insert(before_gesture_recognizer);
    }

    pub fn prioritize_descendant_pans(&mut self, in_root_view: impl IntoIterator<Item = ElementId>) {
        for pan in in_root_view {
            if self.prioritized_pans.contains(&pan) {
                continue;
            }
            self.prioritized_pans.insert(pan);
        }
    }

    pub fn gesture_recognizer_should_begin(&self, velocity: Point<Pixels>) -> bool {
        let horizontal = f32::from(velocity.x).abs();
        let vertical = f32::from(velocity.y).abs();
        if vertical <= 0.0 {
            return false;
        }
        vertical >= horizontal * 0.85
    }

    pub fn handle_scroll_wheel(&mut self, event: &ScrollWheelEvent) {
        if event.touch_phase == TouchPhase::Started || matches!(event.delta, ScrollDelta::Lines(_)) {
            self.vertical_intent = None;
        }
        if self.vertical_intent.is_none() {
            let velocity = Self::delta_point(&event.delta);
            if f32::from(velocity.x) != 0.0 || f32::from(velocity.y) != 0.0 {
                self.vertical_intent = Some(self.gesture_recognizer_should_begin(velocity));
            }
        }
    }

    pub fn allows_pan(&self, pan: &ElementId) -> bool {
        if !self.prioritized_pans.contains(pan) {
            return true;
        }
        self.vertical_intent == Some(false)
    }

    fn delta_point(delta: &ScrollDelta) -> Point<Pixels> {
        match delta {
            ScrollDelta::Pixels(point) => *point,
            ScrollDelta::Lines(point) => Point {
                x: px(point.x),
                y: px(point.y),
            },
        }
    }
}

/// the enclosing scroll of the current gesture is cancelled, the list ignores the rest of it
#[derive(Default)]
struct RiseEnclosingScrollPanCancellation {
    is_cancelled: bool,
}

impl Global for RiseEnclosingScrollPanCancellation {}

/// a reply swipe owns the gesture once it starts, the list under the row stays put until it ends
/// only the current gesture is cancelled
pub fn cancel_enclosing_scroll_pan_for_current_touch(cx: &mut App) {
    cx.stop_propagation();
    cx.default_global::<RiseEnclosingScrollPanCancellation>().is_cancelled = true;
}

pub fn enclosing_scroll_pan_is_cancelled_for_current_touch(event: &ScrollWheelEvent, cx: &mut App) -> bool {
    let is_cancelled = cx
        .try_global::<RiseEnclosingScrollPanCancellation>()
        .is_some_and(|cancellation| cancellation.is_cancelled);
    if is_cancelled && matches!(event.touch_phase, TouchPhase::Ended | TouchPhase::Cancelled) {
        cx.default_global::<RiseEnclosingScrollPanCancellation>().is_cancelled = false;
    }
    is_cancelled
}
