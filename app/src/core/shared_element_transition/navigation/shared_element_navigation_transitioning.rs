use std::cell::RefCell;
use std::rc::{Rc, Weak};

use gpui::{App, Bounds, Pixels, Point};

use crate::core::shared_element_transition::navigation::right_to_left_interactive_pop::RightToLeftInteractivePop;
use crate::core::shared_element_transition::navigation::shared_element_navigation_delegate_proxy::SharedElementNavigationDelegateProxy;
use crate::core::shared_element_transition::renderer::shared_element_flight_runner::SharedElementFlightContainer;
use crate::core::shared_element_transition::runtime::shared_element_host_probe::SharedElementStackHost;

/// one page of the route stack as the transition layer sees it
pub type SharedElementNavigationHost = SharedElementStackHost;

/// navigation operation, push or pop
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SharedElementNavigationOperation {
    None,
    Push,
    Pop,
}

impl SharedElementNavigationOperation {
    pub fn raw_value(self) -> i64 {
        match self {
            Self::None => 0,
            Self::Push => 1,
            Self::Pop => 2,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SharedElementTransitionContextKey {
    From,
    To,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SharedElementPanState {
    Possible,
    Began,
    Changed,
    Ended,
    Cancelled,
    Failed,
}

/// pan gesture, the trackpad swipe or pointer drag that pops the top screen
/// points are in window space and on desktop the window is the screen of the transition
pub trait SharedElementPanGesture {
    fn state(&self) -> SharedElementPanState;
    /// translation in window space
    fn translation(&self) -> Point<Pixels>;
    /// velocity in points per second
    fn velocity(&self) -> Point<Pixels>;
    /// location in window space
    fn location(&self) -> Point<Pixels>;
    fn view_bounds(&self) -> Option<Bounds<Pixels>>;
}

/// the two screens of a running push or pop
/// the stack lays both out in its container while the context lives and views are named by key
pub trait SharedElementViewControllerContextTransitioning {
    fn container_bounds(&self, cx: &App) -> Bounds<Pixels>;
    fn container_view(&self, cx: &mut App) -> Rc<dyn SharedElementFlightContainer>;
    fn view_flight_container(
        &self,
        key: SharedElementTransitionContextKey,
        cx: &mut App,
    ) -> Option<Rc<dyn SharedElementFlightContainer>>;
    fn add_touch_watch(
        &self,
        key: SharedElementTransitionContextKey,
        on_touch_began: Rc<dyn Fn(&mut App)>,
        cx: &mut App,
    ) -> SharedElementUserInputWatch;
    fn view_controller(&self, key: SharedElementTransitionContextKey) -> Option<SharedElementNavigationHost>;
    fn apply_final_frame(&self, key: SharedElementTransitionContextKey, cx: &mut App);
    fn add_subview(&self, key: SharedElementTransitionContextKey, cx: &mut App);
    /// inserts the view below a sibling in the container
    fn insert_subview_below(
        &self,
        key: SharedElementTransitionContextKey,
        sibling: SharedElementTransitionContextKey,
        cx: &mut App,
    );
    fn remove_from_superview(&self, key: SharedElementTransitionContextKey, cx: &mut App);
    fn alpha(&self, key: SharedElementTransitionContextKey, cx: &App) -> f32;
    fn set_alpha(&self, key: SharedElementTransitionContextKey, alpha: f32, cx: &mut App);
    /// sets the x translation of the view, zero is identity
    fn set_translation_x(&self, key: SharedElementTransitionContextKey, translation: f32, cx: &mut App);
    /// turns user interaction on the view on or off
    fn set_user_interaction_enabled(&self, key: SharedElementTransitionContextKey, enabled: bool, cx: &mut App);
    /// lays out the view if needed
    fn layout_if_needed(&self, key: SharedElementTransitionContextKey, cx: &mut App);
    fn update_interactive_transition(&self, percent_complete: f32, cx: &mut App);
    fn finish_interactive_transition(&self, cx: &mut App);
    fn cancel_interactive_transition(&self, cx: &mut App);
    fn complete_transition(&self, did_complete: bool, cx: &mut App);
    fn transition_was_cancelled(&self, cx: &App) -> bool;
}

/// an installed touch watch, remove takes it off the view
pub struct SharedElementUserInputWatch {
    remove: RefCell<Option<Box<dyn FnOnce(&mut App)>>>,
}

impl SharedElementUserInputWatch {
    pub fn new(remove: impl FnOnce(&mut App) + 'static) -> Self {
        Self { remove: RefCell::new(Some(Box::new(remove))) }
    }

    pub fn remove(&self, cx: &mut App) {
        let remove = self.remove.borrow_mut().take();
        if let Some(remove) = remove {
            remove(cx);
        }
    }
}

/// private target of the stack own interactive pop
/// a stock mode back pan is forwarded to it unchanged
pub trait SharedElementStockPopTarget {
    fn handle_navigation_transition(&self, pan: &Rc<dyn SharedElementPanGesture>, cx: &mut App);
}

/// transition coordinator of the running stock transition
pub trait SharedElementViewControllerTransitionCoordinator {
    fn view_controller(&self, key: SharedElementTransitionContextKey) -> Option<SharedElementNavigationHost>;
    fn flight_container(
        &self,
        key: SharedElementTransitionContextKey,
        cx: &mut App,
    ) -> Option<Rc<dyn SharedElementFlightContainer>>;
    fn animate_alongside_transition(
        &self,
        animation: Rc<dyn Fn(f32, &mut App)>,
        completion: Box<dyn FnOnce(&mut App)>,
        cx: &mut App,
    ) -> bool;
}

pub trait SharedElementViewControllerAnimatedTransitioning {
    fn transition_duration(&self, transition_context: Option<&Rc<dyn SharedElementViewControllerContextTransitioning>>) -> f64;
    fn animate_transition(&self, transition_context: Rc<dyn SharedElementViewControllerContextTransitioning>, cx: &mut App);
    fn as_right_to_left_pop(self: Rc<Self>) -> Option<Rc<RightToLeftInteractivePop>> {
        None
    }
}

pub trait SharedElementViewControllerInteractiveTransitioning {
    fn start_interactive_transition(
        &self,
        transition_context: Rc<dyn SharedElementViewControllerContextTransitioning>,
        cx: &mut App,
    );
    fn wants_interactive_start(&self) -> bool {
        true
    }
}

/// navigation controller as the shared element navigation uses it
pub trait SharedElementNavigationController {
    fn top_view_controller(&self, cx: &App) -> Option<SharedElementNavigationHost>;
    fn pop_view_controller(&self, animated: bool, cx: &mut App);
    /// transition coordinator while a push or pop runs
    fn transition_coordinator(&self, cx: &App) -> Option<Rc<dyn SharedElementViewControllerTransitionCoordinator>>;
    fn delegate(&self) -> Option<Rc<dyn SharedElementNavigationControllerDelegate>>;
    fn set_delegate(&self, delegate: Option<Rc<dyn SharedElementNavigationControllerDelegate>>);
    fn associated_delegate_proxy(&self) -> Option<Rc<SharedElementNavigationDelegateProxy>>;
    fn set_associated_delegate_proxy(&self, proxy: Option<Rc<SharedElementNavigationDelegateProxy>>);
    fn set_interactive_content_pop_enabled(&self, enabled: bool, cx: &mut App);
    fn preferred_transition_is_nil(&self, controller: SharedElementNavigationHost, cx: &App) -> bool;
}

/// navigation controller delegate, every requirement is optional
pub trait SharedElementNavigationControllerDelegate {
    fn navigation_controller_will_show(
        &self,
        _navigation_controller: &Rc<dyn SharedElementNavigationController>,
        _view_controller: SharedElementNavigationHost,
        _animated: bool,
        _cx: &mut App,
    ) {
    }

    fn navigation_controller_did_show(
        &self,
        _navigation_controller: &Rc<dyn SharedElementNavigationController>,
        _view_controller: SharedElementNavigationHost,
        _animated: bool,
        _cx: &mut App,
    ) {
    }

    fn navigation_controller_animation_controller_for(
        &self,
        _navigation_controller: &Rc<dyn SharedElementNavigationController>,
        _operation: SharedElementNavigationOperation,
        _from: SharedElementNavigationHost,
        _to: SharedElementNavigationHost,
        _cx: &mut App,
    ) -> Option<Rc<dyn SharedElementViewControllerAnimatedTransitioning>> {
        None
    }

    fn navigation_controller_interaction_controller_for(
        &self,
        _navigation_controller: &Rc<dyn SharedElementNavigationController>,
        _animation_controller: &Rc<dyn SharedElementViewControllerAnimatedTransitioning>,
        _cx: &mut App,
    ) -> Option<Rc<dyn SharedElementViewControllerInteractiveTransitioning>> {
        None
    }

    fn as_delegate_proxy(&self) -> Option<&SharedElementNavigationDelegateProxy> {
        None
    }
}

/// weak ref to one view of a transition context, held by an animator
#[derive(Clone)]
pub struct SharedElementTransitionView {
    context: Weak<dyn SharedElementViewControllerContextTransitioning>,
    key: SharedElementTransitionContextKey,
}

impl SharedElementTransitionView {
    pub fn of(
        context: &Rc<dyn SharedElementViewControllerContextTransitioning>,
        key: SharedElementTransitionContextKey,
    ) -> Option<Self> {
        context.view_controller(key)?;
        Some(Self { context: Rc::downgrade(context), key })
    }

    pub fn key(&self) -> SharedElementTransitionContextKey {
        self.key
    }

    fn context(&self) -> Option<Rc<dyn SharedElementViewControllerContextTransitioning>> {
        self.context.upgrade()
    }

    pub fn alpha(&self, cx: &App) -> f32 {
        self.context().map_or(1.0, |context| context.alpha(self.key, cx))
    }

    pub fn set_alpha(&self, alpha: f32, cx: &mut App) {
        if let Some(context) = self.context() {
            context.set_alpha(self.key, alpha, cx);
        }
    }

    pub fn set_translation_x(&self, translation: f32, cx: &mut App) {
        if let Some(context) = self.context() {
            context.set_translation_x(self.key, translation, cx);
        }
    }

    pub fn set_user_interaction_enabled(&self, enabled: bool, cx: &mut App) {
        if let Some(context) = self.context() {
            context.set_user_interaction_enabled(self.key, enabled, cx);
        }
    }

    pub fn layout_if_needed(&self, cx: &mut App) {
        if let Some(context) = self.context() {
            context.layout_if_needed(self.key, cx);
        }
    }

    pub fn remove_from_superview(&self, cx: &mut App) {
        if let Some(context) = self.context() {
            context.remove_from_superview(self.key, cx);
        }
    }
}
