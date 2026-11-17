use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{App, Window};
use rise_core::foundation::foundation_date::RiseClock;
use rise_platform::display_cadence::maximum_frames_per_second;
use rise_platform::power::system_low_power_mode;

/// the frame rate range an animation asks the display for
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RiseFrameRateRange {
    pub minimum: f32,
    pub maximum: f32,
    pub preferred: f32,
}

pub fn rise_use_display_cadence(window: &Window) -> Option<RiseFrameRateRange> {
    let cadence = maximum_frames_per_second(window) as f32;
    if cadence <= 61.0 {
        return None;
    }
    Some(RiseFrameRateRange {
        minimum: 30.0,
        maximum: cadence,
        preferred: cadence,
    })
}

pub struct RiseMotionCadencePolicy;

impl RiseMotionCadencePolicy {
    pub fn preferred_fps(maximum_fps: i64, low_power: bool) -> f32 {
        let limit = if low_power { 60 } else { maximum_fps };
        maximum_fps.min(limit).max(1) as f32
    }
}

const MAXIMUM_REQUEST: Duration = Duration::from_secs(2);

pub type FrameHandler = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(Default)]
struct CadenceState {
    is_linked: bool,
    generation: u64,
    deadline: Duration,
    on_frame: Option<FrameHandler>,
    frame_interval: Duration,
    last_frame: Option<Duration>,
    was_active: bool,
}

/// transitions dont expose their animations so display cadence is requested only for their
/// bounded motion interval
/// an idle menu must not keep a high refresh display awake
#[derive(Clone, Default)]
pub struct RiseMotionCadence {
    state: Rc<RefCell<CadenceState>>,
}

impl RiseMotionCadence {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn request(
        &self,
        duration: Duration,
        on_frame: Option<FrameHandler>,
        window: &mut Window,
        _cx: &mut App,
    ) {
        if duration.is_zero() {
            return;
        }
        let generation = {
            let mut state = self.state.borrow_mut();
            let deadline = RiseClock::monotonic() + duration.min(MAXIMUM_REQUEST);
            state.deadline = state.deadline.max(deadline);
            if let Some(on_frame) = on_frame {
                state.on_frame = Some(on_frame);
            }
            if state.is_linked {
                return;
            }
            let maximum_fps = i64::from(maximum_frames_per_second(window));
            let fps = RiseMotionCadencePolicy::preferred_fps(
                maximum_fps,
                system_low_power_mode(),
            );
            state.frame_interval = Duration::from_secs_f32(1.0 / fps);
            state.last_frame = None;
            state.was_active = window.is_window_active();
            state.is_linked = true;
            state.generation
        };
        Self::schedule(Rc::clone(&self.state), generation, window);
    }

    pub fn stop(&self) {
        let mut state = self.state.borrow_mut();
        state.is_linked = false;
        state.generation = state.generation.wrapping_add(1);
        state.deadline = Duration::ZERO;
        state.on_frame = None;
        state.last_frame = None;
    }

    fn schedule(state: Rc<RefCell<CadenceState>>, generation: u64, window: &mut Window) {
        window.on_next_frame(move |window, cx| Self::tick(state, generation, window, cx));
    }

    fn tick(state: Rc<RefCell<CadenceState>>, generation: u64, window: &mut Window, cx: &mut App) {
        let now = RiseClock::monotonic();
        let frame = {
            let mut current = state.borrow_mut();
            if !current.is_linked || current.generation != generation {
                return;
            }
            let is_active = window.is_window_active();
            let resigned = current.was_active && !is_active;
            current.was_active = is_active;
            if resigned {
                drop(current);
                Self { state }.stop();
                return;
            }
            let due = current
                .last_frame
                .is_none_or(|last| now.saturating_sub(last) >= current.frame_interval);
            if due {
                current.last_frame = Some(now);
                Some(current.on_frame.clone())
            } else {
                None
            }
        };
        match frame {
            Some(Some(handler)) => handler(window, cx),
            Some(None) => window.refresh(),
            None => {}
        }
        let expired = {
            let current = state.borrow();
            if !current.is_linked || current.generation != generation {
                return;
            }
            RiseClock::monotonic() >= current.deadline
        };
        if expired {
            Self { state }.stop();
        } else {
            Self::schedule(state, generation, window);
        }
    }
}
