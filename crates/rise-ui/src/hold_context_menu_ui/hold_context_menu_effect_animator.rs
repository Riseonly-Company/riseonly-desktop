use std::rc::Rc;

use gpui::{App, Window};

use crate::swiftui::animated::{Animated, MotionTime};
use crate::swiftui::motion::ease_out;

/// runs once the effect rests on the requested endpoint
pub type HoldContextMenuEffectCompletion = Rc<dyn Fn(&mut Window, &mut App)>;

/// an effect has two valid endpoints
/// a running transition is reversed from where it is on screen, not finished into a half built effect
pub struct HoldContextMenuEffectAnimator {
    progress: Animated<f32>,
    animator_running: bool,
    target: Option<bool>,
    animation_end: bool,
    generation: u64,
    notification: u64,
    completion: Option<HoldContextMenuEffectCompletion>,
    pending_notification: Option<u64>,
}

impl Default for HoldContextMenuEffectAnimator {
    fn default() -> Self {
        Self::new()
    }
}

impl HoldContextMenuEffectAnimator {
    pub fn new() -> Self {
        Self {
            progress: Animated::new(0.0),
            animator_running: false,
            target: None,
            animation_end: false,
            generation: 0,
            notification: 0,
            completion: None,
            pending_notification: None,
        }
    }

    /// sets the revealed state, optionally animated
    /// now and reduce_motion stand in for the clock and the system reduce motion flag
    pub fn set_revealed(
        &mut self,
        revealed: bool,
        animated: bool,
        duration: f64,
        completion: Option<HoldContextMenuEffectCompletion>,
        now: MotionTime,
        reduce_motion: bool,
    ) {
        self.completion = completion;
        self.notification += 1;
        if self.target == Some(revealed) {
            if !self.animator_running {
                self.notify_completion();
            }
            return;
        }
        let has_initial_value = self.target.is_some();
        self.target = Some(revealed);
        let end_value = if revealed { 1.0 } else { 0.0 };
        if !(animated && has_initial_value && !reduce_motion) {
            self.cancel_animator();
            self.progress.jump(end_value);
            self.notify_completion();
            return;
        }
        if self.animator_running && self.progress.is_running(now) {
            let covered = (self.progress.value(now) - end_value).abs() as f64;
            self.animation_end = revealed;
            self.progress.set(end_value, ease_out(duration * covered.clamp(0.0, 1.0)), now, false);
            return;
        }

        self.cancel_animator();
        self.animation_end = revealed;
        self.animator_running = true;
        self.progress.set(end_value, ease_out(duration), now, false);
    }

    pub fn cancel(&mut self) {
        self.completion = None;
        self.notification += 1;
        self.cancel_animator();
    }

    /// fraction on screen at now
    /// due completion fires after this frame and never on the render call stack
    pub fn revealed_fraction(&mut self, now: MotionTime, window: &mut Window, cx: &mut App) -> f32 {
        let value = self.progress.frame_value(now, window);
        if self.animator_running && !self.progress.is_running(now) {
            self.animator_running = false;
            self.notify_completion();
        }
        if let Some(notification) = self.pending_notification.take()
            && notification == self.notification
            && !self.animator_running
            && let Some(completion) = self.completion.take()
        {
            window.defer(cx, move |window, cx| completion(window, cx));
        }
        value
    }

    pub fn target(&self) -> Option<bool> {
        self.target
    }

    pub fn animation_end(&self) -> bool {
        self.animation_end
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    fn cancel_animator(&mut self) {
        self.generation += 1;
        self.animator_running = false;
    }

    fn notify_completion(&mut self) {
        if self.completion.is_none() {
            return;
        }
        self.pending_notification = Some(self.notification);
    }
}
