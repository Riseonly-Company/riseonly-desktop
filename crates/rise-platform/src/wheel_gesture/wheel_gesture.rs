use std::time::Duration;

use gpui::{Pixels, Point, ScrollWheelEvent, TouchPhase};

use crate::host_os::HostOs;

pub const WHEEL_SETTLE_DELAY: Duration = Duration::from_millis(150);

/// what one scroll event did to the gesture
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WheelStep {
    Ignored,
    Moved(Point<Pixels>),
    Ended(Point<Pixels>),
    Cancelled,
}

/// proof of the newest unphased scroll event, settle acts on it only while no newer event came
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WheelSettleToken(u64);

/// result of WheelGestureTracker feed
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WheelFeed {
    pub step: WheelStep,
    pub settle: Option<WheelSettleToken>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Session {
    Idle,
    Phased { translation: Point<Pixels> },
    Unphased { translation: Point<Pixels> },
    Coasting,
}

/// turns scroll events into one swipe with a start and an end
/// one tracker per surface that follows the wheel, keep it in the surface state
#[derive(Clone, Debug)]
pub struct WheelGestureTracker {
    session: Session,
    generation: u64,
}

impl Default for WheelGestureTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl WheelGestureTracker {
    pub const fn new() -> Self {
        Self { session: Session::Idle, generation: 0 }
    }

    /// true while a gesture is followed, not idle and not coasting after an end
    pub fn is_tracking(&self) -> bool {
        matches!(self.session, Session::Phased { .. } | Session::Unphased { .. })
    }

    /// forgets the gesture without an end, for a surface that went away or got taken by a drag
    /// a pending settle token does nothing after
    pub fn reset(&mut self) {
        self.session = Session::Idle;
        self.generation = self.generation.wrapping_add(1);
    }

    /// feeds one scroll event, line_height turns mouse wheel line deltas into pixels
    pub fn feed(&mut self, event: &ScrollWheelEvent, line_height: Pixels) -> WheelFeed {
        let delta = event.delta.pixel_delta(line_height);
        if !event.delta.precise() {
            return self.unphased(delta);
        }
        match event.touch_phase {
            TouchPhase::Started => {
                self.generation = self.generation.wrapping_add(1);
                self.session = Session::Phased { translation: delta };
                WheelFeed { step: WheelStep::Moved(delta), settle: None }
            }
            TouchPhase::Moved => match self.session {
                Session::Phased { translation } => {
                    let translation = translation + delta;
                    self.session = Session::Phased { translation };
                    WheelFeed { step: WheelStep::Moved(translation), settle: None }
                }
                Session::Coasting => WheelFeed { step: WheelStep::Ignored, settle: Some(self.next_token()) },
                Session::Idle | Session::Unphased { .. } => self.unphased(delta),
            },
            TouchPhase::Ended => match self.session {
                Session::Phased { translation } => {
                    let translation = translation + delta;
                    self.session = Session::Coasting;
                    WheelFeed { step: WheelStep::Ended(translation), settle: Some(self.next_token()) }
                }
                Session::Idle | Session::Unphased { .. } | Session::Coasting => {
                    WheelFeed { step: WheelStep::Ignored, settle: None }
                }
            },
            TouchPhase::Cancelled => match self.session {
                Session::Phased { .. } | Session::Unphased { .. } => {
                    self.reset();
                    WheelFeed { step: WheelStep::Cancelled, settle: None }
                }
                Session::Idle | Session::Coasting => WheelFeed { step: WheelStep::Ignored, settle: None },
            },
        }
    }

    /// called by the feature timer WHEEL_SETTLE_DELAY after the feed that gave token
    /// gives Ended when that feed was the last of an unphased gesture
    /// none when a newer event came or only momentum went quiet
    pub fn settle(&mut self, token: WheelSettleToken) -> Option<WheelStep> {
        if token.0 != self.generation {
            return None;
        }
        match self.session {
            Session::Unphased { translation } => {
                self.session = Session::Idle;
                Some(WheelStep::Ended(translation))
            }
            Session::Coasting => {
                self.session = Session::Idle;
                None
            }
            Session::Idle | Session::Phased { .. } => None,
        }
    }

    fn unphased(&mut self, delta: Point<Pixels>) -> WheelFeed {
        let translation = match self.session {
            Session::Unphased { translation } => translation + delta,
            Session::Idle | Session::Phased { .. } | Session::Coasting => delta,
        };
        self.session = Session::Unphased { translation };
        WheelFeed { step: WheelStep::Moved(translation), settle: Some(self.next_token()) }
    }

    fn next_token(&mut self) -> WheelSettleToken {
        self.generation = self.generation.wrapping_add(1);
        WheelSettleToken(self.generation)
    }
}

/// one burst of Lines events over a pager
/// eg a notched wheel flick, x11 smooth scroll or a windows touchpad swipe
/// turns at most one page
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineBurst {
    accumulated_along: f32,
    accumulated_across: f32,
    last_at: Duration,
    turned: bool,
}

/// what one Lines event does to a pager
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineBurstVerdict {
    NotMine,
    Claimed,
    TurnBack,
    TurnForward,
}

/// travel along the pager axis in lines at which a burst turns its page
pub const fn line_burst_turn_lines(host: HostOs) -> f32 {
    match host {
        HostOs::MacOs => 0.0,
        HostOs::Linux | HostOs::Windows => 1.0,
    }
}

/// folds one Lines event into the current burst, which ends after WHEEL_SETTLE_DELAY with no events
/// turns the page once when travel along the axis hits turn_lines and beats the travel across
/// positive travel turns back, a burst not opened inside the pager is no burst
pub fn line_burst_step(
    burst: Option<LineBurst>,
    along: f32,
    across: f32,
    inside: bool,
    turn_lines: f32,
    now: Duration,
) -> (Option<LineBurst>, LineBurstVerdict) {
    let live = burst.filter(|burst| now.saturating_sub(burst.last_at) < WHEEL_SETTLE_DELAY);
    let mut burst = match live {
        Some(burst) => burst,
        None if inside => LineBurst { accumulated_along: 0.0, accumulated_across: 0.0, last_at: now, turned: false },
        None => return (None, LineBurstVerdict::NotMine),
    };
    burst.accumulated_along += along;
    burst.accumulated_across += across;
    burst.last_at = now;
    let leads = burst.accumulated_along.abs() > burst.accumulated_across.abs();
    let verdict = if !leads || along == 0.0 {
        LineBurstVerdict::NotMine
    } else if burst.turned || burst.accumulated_along.abs() < turn_lines {
        LineBurstVerdict::Claimed
    } else {
        burst.turned = true;
        if burst.accumulated_along > 0.0 { LineBurstVerdict::TurnBack } else { LineBurstVerdict::TurnForward }
    };
    (Some(burst), verdict)
}
