use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use gpui::App;
use rise_core::foundation::foundation_date::RiseClock;

use crate::core::shared_element_transition::kernel::shared_element_motion::SharedElementMotion;
use crate::core::shared_element_transition::kernel::shared_element_types::{
    SharedElementGesturePolicy, SharedElementGesturePolicyAxis, SharedElementGesturePolicyDecision,
    SharedElementGesturePolicyInput,
};
use crate::core::shared_element_transition::navigation::shared_element_navigation_driver::SharedElementFrameTicker;
use crate::core::shared_element_transition::navigation::shared_element_navigation_transitioning::{
    SharedElementPanGesture, SharedElementPanState, SharedElementTransitionContextKey, SharedElementTransitionView,
    SharedElementViewControllerAnimatedTransitioning, SharedElementViewControllerContextTransitioning,
    SharedElementViewControllerInteractiveTransitioning,
};

/// back swipe for a right to left language, top screen leaves to
/// the left and the previous comes from the right under the finger
/// the desktop stack has no direction of its own
pub struct RightToLeftInteractivePop {
    this: Weak<Self>,
    context: RefCell<Option<Rc<dyn SharedElementViewControllerContextTransitioning>>>,
    from_view: RefCell<Option<SharedElementTransitionView>>,
    to_view: RefCell<Option<SharedElementTransitionView>>,
    width: Cell<f32>,
    percent: Cell<f32>,
    ticker: SharedElementFrameTicker,
}

impl RightToLeftInteractivePop {
    const PARALLAX: f32 = 0.3;

    pub fn new() -> Rc<Self> {
        Rc::new_cyclic(|this| Self {
            this: this.clone(),
            context: RefCell::new(None),
            from_view: RefCell::new(None),
            to_view: RefCell::new(None),
            width: Cell::new(0.0),
            percent: Cell::new(0.0),
            ticker: SharedElementFrameTicker::new(),
        })
    }

    fn context(&self) -> Option<Rc<dyn SharedElementViewControllerContextTransitioning>> {
        self.context.borrow().clone()
    }

    /// linear frame of percent for both screens
    fn apply(&self, percent: f32, cx: &mut App) {
        let width = self.width.get();
        let from_view = self.from_view.borrow().clone();
        let to_view = self.to_view.borrow().clone();
        if let Some(from_view) = from_view {
            from_view.set_translation_x(-width * percent, cx);
        }
        if let Some(to_view) = to_view {
            to_view.set_translation_x(width * Self::PARALLAX * (1.0 - percent), cx);
        }
    }

    pub fn update(&self, percent_complete: f32, cx: &mut App) {
        let Some(context) = self.context() else { return };
        self.percent.set(percent_complete);
        self.apply(percent_complete, cx);
        context.update_interactive_transition(percent_complete, cx);
    }

    /// finishes the interactive transition
    pub fn finish(&self, cx: &mut App) {
        let Some(context) = self.context() else { return };
        context.finish_interactive_transition(cx);
        self.run_to(1.0, cx);
    }

    /// cancels the interactive transition
    pub fn cancel(&self, cx: &mut App) {
        let Some(context) = self.context() else { return };
        context.cancel_interactive_transition(cx);
        self.run_to(0.0, cx);
    }

    /// the paused linear animation resumes from the current percent at its own speed
    fn run_to(&self, target: f32, cx: &mut App) {
        let start = self.percent.get();
        let remaining = (target - start).abs() as f64 * self.transition_duration(None);
        let started_at = RiseClock::monotonic();
        let this = self.this.clone();
        self.ticker.start(
            move |cx: &mut App| {
                let Some(this) = this.upgrade() else { return };
                let elapsed = RiseClock::monotonic().saturating_sub(started_at).as_secs_f64();
                let fraction = if remaining > 0.0 { (elapsed / remaining).min(1.0) as f32 } else { 1.0 };
                let percent = start + (target - start) * fraction;
                this.percent.set(percent);
                this.apply(percent, cx);
                if fraction >= 1.0 {
                    this.ticker.stop();
                    this.animation_did_complete(cx);
                }
            },
            cx,
        );
    }

    fn animation_did_complete(&self, cx: &mut App) {
        let from_view = self.from_view.borrow_mut().take();
        let to_view = self.to_view.borrow_mut().take();
        if let Some(from_view) = from_view {
            from_view.set_translation_x(0.0, cx);
        }
        if let Some(to_view) = to_view {
            to_view.set_translation_x(0.0, cx);
        }
        let Some(context) = self.context.borrow_mut().take() else { return };
        let did_complete = !context.transition_was_cancelled(cx);
        context.complete_transition(did_complete, cx);
    }

    pub fn handle(&self, pan: &dyn SharedElementPanGesture, cx: &mut App) {
        let viewport = pan.view_bounds().map(|bounds| bounds.size).unwrap_or_default();
        let translation = pan.translation();
        match pan.state() {
            SharedElementPanState::Changed => {
                let width = f32::from(viewport.width);
                if width <= 0.0 {
                    return;
                }
                self.update((-f32::from(translation.x) / width).clamp(0.0, 1.0), cx);
            }
            SharedElementPanState::Ended => {
                let decision = SharedElementGesturePolicy::decision(&SharedElementGesturePolicyInput {
                    translation,
                    velocity: pan.velocity(),
                    touch_start: Default::default(),
                    source: Default::default(),
                    viewport,
                    layout_sign: -1.0,
                    axis: SharedElementGesturePolicyAxis::Horizontal,
                });
                match decision {
                    SharedElementGesturePolicyDecision::Finish => self.finish(cx),
                    SharedElementGesturePolicyDecision::Cancel => self.cancel(cx),
                }
            }
            SharedElementPanState::Cancelled | SharedElementPanState::Failed => self.cancel(cx),
            SharedElementPanState::Possible | SharedElementPanState::Began => {}
        }
    }
}

impl SharedElementViewControllerAnimatedTransitioning for RightToLeftInteractivePop {
    fn transition_duration(&self, _: Option<&Rc<dyn SharedElementViewControllerContextTransitioning>>) -> f64 {
        SharedElementMotion::CANCEL_DURATION
    }

    fn animate_transition(&self, context: Rc<dyn SharedElementViewControllerContextTransitioning>, cx: &mut App) {
        let (Some(from_view), Some(to_view)) = (
            SharedElementTransitionView::of(&context, SharedElementTransitionContextKey::From),
            SharedElementTransitionView::of(&context, SharedElementTransitionContextKey::To),
        ) else {
            context.complete_transition(false, cx);
            return;
        };
        self.width.set(f32::from(context.container_bounds(cx).size.width));
        context.apply_final_frame(SharedElementTransitionContextKey::To, cx);
        context.insert_subview_below(SharedElementTransitionContextKey::To, SharedElementTransitionContextKey::From, cx);
        *self.from_view.borrow_mut() = Some(from_view);
        *self.to_view.borrow_mut() = Some(to_view);
        *self.context.borrow_mut() = Some(context);
        self.percent.set(0.0);
        self.apply(0.0, cx);
    }

    fn as_right_to_left_pop(self: Rc<Self>) -> Option<Rc<RightToLeftInteractivePop>> {
        Some(self)
    }
}

impl SharedElementViewControllerInteractiveTransitioning for RightToLeftInteractivePop {
    fn start_interactive_transition(
        &self,
        transition_context: Rc<dyn SharedElementViewControllerContextTransitioning>,
        cx: &mut App,
    ) {
        self.animate_transition(transition_context, cx);
    }
}

impl Drop for RightToLeftInteractivePop {
    fn drop(&mut self) {
        self.ticker.stop();
    }
}
