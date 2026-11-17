use std::cell::RefCell;
use std::rc::{Rc, Weak};

use gpui::App;

use crate::core::shared_element_transition::navigation::shared_element_navigation_driver::SharedElementNavigationDriver;
use crate::core::shared_element_transition::navigation::shared_element_navigation_transitioning::{
    SharedElementNavigationController, SharedElementNavigationControllerDelegate, SharedElementNavigationHost,
    SharedElementNavigationOperation, SharedElementViewControllerAnimatedTransitioning,
    SharedElementViewControllerInteractiveTransitioning,
};

/// delegate of the route stack, answers transition
/// callbacks from the driver first and then from base
/// retained by association on the stack because the stack delegate is weak
pub struct SharedElementNavigationDelegateProxy {
    base: RefCell<Option<Weak<dyn SharedElementNavigationControllerDelegate>>>,
}

impl SharedElementNavigationDelegateProxy {
    pub fn new(base: Option<&Rc<dyn SharedElementNavigationControllerDelegate>>) -> Self {
        Self { base: RefCell::new(base.map(Rc::downgrade)) }
    }

    pub fn base(&self) -> Option<Rc<dyn SharedElementNavigationControllerDelegate>> {
        self.base.borrow().as_ref().and_then(Weak::upgrade)
    }

    pub fn set_base(&self, base: Option<&Rc<dyn SharedElementNavigationControllerDelegate>>) {
        *self.base.borrow_mut() = base.map(Rc::downgrade);
    }
}

impl SharedElementNavigationControllerDelegate for SharedElementNavigationDelegateProxy {
    fn navigation_controller_will_show(
        &self,
        navigation_controller: &Rc<dyn SharedElementNavigationController>,
        view_controller: SharedElementNavigationHost,
        animated: bool,
        cx: &mut App,
    ) {
        SharedElementNavigationDriver::shared().will_show(navigation_controller, view_controller, animated, cx);
        if let Some(base) = self.base() {
            base.navigation_controller_will_show(navigation_controller, view_controller, animated, cx);
        }
    }

    fn navigation_controller_did_show(
        &self,
        navigation_controller: &Rc<dyn SharedElementNavigationController>,
        view_controller: SharedElementNavigationHost,
        animated: bool,
        cx: &mut App,
    ) {
        SharedElementNavigationDriver::shared().did_show(navigation_controller, view_controller, animated, cx);
        if let Some(base) = self.base() {
            base.navigation_controller_did_show(navigation_controller, view_controller, animated, cx);
        }
    }

    fn navigation_controller_animation_controller_for(
        &self,
        navigation_controller: &Rc<dyn SharedElementNavigationController>,
        operation: SharedElementNavigationOperation,
        from: SharedElementNavigationHost,
        to: SharedElementNavigationHost,
        cx: &mut App,
    ) -> Option<Rc<dyn SharedElementViewControllerAnimatedTransitioning>> {
        if let Some(own) = SharedElementNavigationDriver::shared().animation_controller(operation, from, to, cx) {
            return Some(own);
        }
        self.base()?.navigation_controller_animation_controller_for(navigation_controller, operation, from, to, cx)
    }

    fn navigation_controller_interaction_controller_for(
        &self,
        navigation_controller: &Rc<dyn SharedElementNavigationController>,
        animation_controller: &Rc<dyn SharedElementViewControllerAnimatedTransitioning>,
        cx: &mut App,
    ) -> Option<Rc<dyn SharedElementViewControllerInteractiveTransitioning>> {
        if let Some(own) = SharedElementNavigationDriver::shared().interaction_controller(animation_controller, cx) {
            return Some(own);
        }
        self.base()?.navigation_controller_interaction_controller_for(navigation_controller, animation_controller, cx)
    }

    fn as_delegate_proxy(&self) -> Option<&SharedElementNavigationDelegateProxy> {
        Some(self)
    }
}
