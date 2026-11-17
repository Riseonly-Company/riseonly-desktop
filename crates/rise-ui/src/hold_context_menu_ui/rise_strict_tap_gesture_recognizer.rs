use std::rc::Rc;

use gpui::{Pixels, Point};

/// the one movement budget every chat tap is measured against
/// a plain click accepts a press however far it drifted
/// so a press that moved with a scrolling list looks like a click
pub enum RiseTapPolicy {}

impl RiseTapPolicy {
    pub const MOVEMENT_SLOP: f32 = 6.0;
}

/// true when the list a tap must lose to is moving, dragged or still gliding
pub type RiseScrollMotionProbe = Rc<dyn Fn() -> bool>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiseStrictTapGestureRecognizerState {
    Possible,
    Recognized,
    Failed,
}

/// tap that refuses two presses a plain click accepts
/// one that moved past movement_slop in window space and one that landed on an already moving list
/// the second is a stop gesture
pub struct RiseStrictTapGestureRecognizer {
    pub governing_scroll_view: Option<RiseScrollMotionProbe>,
    pub movement_slop: f32,
    state: RiseStrictTapGestureRecognizerState,
    start_window_location: Option<Point<Pixels>>,
    resolved_scroll_view: Option<RiseScrollMotionProbe>,
}

impl Default for RiseStrictTapGestureRecognizer {
    fn default() -> Self {
        Self::new()
    }
}

impl RiseStrictTapGestureRecognizer {
    pub fn new() -> Self {
        Self {
            governing_scroll_view: None,
            movement_slop: RiseTapPolicy::MOVEMENT_SLOP,
            state: RiseStrictTapGestureRecognizerState::Possible,
            start_window_location: None,
            resolved_scroll_view: None,
        }
    }

    pub fn state(&self) -> RiseStrictTapGestureRecognizerState {
        self.state
    }

    pub fn reset(&mut self) {
        self.state = RiseStrictTapGestureRecognizerState::Possible;
        self.start_window_location = None;
        self.resolved_scroll_view = None;
    }

    pub fn touches_began(&mut self, window_location: Point<Pixels>) {
        self.reset();
        let scroll_view = self.governing_scroll_view.clone();
        self.start_window_location = Some(window_location);
        if scroll_view.as_ref().is_some_and(|is_moving| is_moving()) {
            self.state = RiseStrictTapGestureRecognizerState::Failed;
        }
        self.resolved_scroll_view = scroll_view;
    }

    pub fn touches_moved(&mut self, window_location: Point<Pixels>) {
        if self.state != RiseStrictTapGestureRecognizerState::Possible {
            return;
        }
        let Some(start) = self.start_window_location else { return };
        let dx = f32::from(window_location.x - start.x);
        let dy = f32::from(window_location.y - start.y);
        if dx * dx + dy * dy <= self.movement_slop * self.movement_slop {
            return;
        }
        self.state = RiseStrictTapGestureRecognizerState::Failed;
    }

    /// pointer up, true when the press counts as a tap
    pub fn touches_ended(&mut self, window_location: Point<Pixels>) -> bool {
        self.touches_moved(window_location);
        if self.state == RiseStrictTapGestureRecognizerState::Possible
            && self.resolved_scroll_view.as_ref().is_some_and(|is_moving| is_moving())
        {
            self.state = RiseStrictTapGestureRecognizerState::Failed;
            return false;
        }
        if self.state != RiseStrictTapGestureRecognizerState::Possible || self.start_window_location.is_none() {
            return false;
        }
        self.state = RiseStrictTapGestureRecognizerState::Recognized;
        true
    }
}
