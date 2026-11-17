use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use gpui::App;

use crate::app::router::navigation_fullscreen_pop::NavigationInteractivePopGuard;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;
use crate::core::shared_element_transition::kernel::shared_element_types::{
    SharedElementGesturePolicyAxis, SharedElementPopPanMode,
};
use crate::core::shared_element_transition::navigation::right_to_left_interactive_pop::RightToLeftInteractivePop;
use crate::core::shared_element_transition::navigation::shared_element_navigation_driver::SharedElementNavigationDriver;
use crate::core::shared_element_transition::navigation::shared_element_navigation_transitioning::{
    SharedElementNavigationController, SharedElementPanGesture, SharedElementPanState, SharedElementStockPopTarget,
};

/// only target of the full screen back pan, latch sets a mode per gesture
/// modes Stock and StockArmed forward all states to the stacks pop target
/// modes Flight and Crossfade feed the driver
pub struct SharedElementPopPanRouter {
    navigation_controller: Weak<dyn SharedElementNavigationController>,
    private_target: Weak<dyn SharedElementStockPopTarget>,
    latched_mode: Cell<SharedElementPopPanMode>,
    active_mode: Cell<SharedElementPopPanMode>,
    right_to_left_pop: RefCell<Option<Rc<RightToLeftInteractivePop>>>,
}

impl SharedElementPopPanRouter {
    pub fn new(
        navigation_controller: &Rc<dyn SharedElementNavigationController>,
        private_target: &Rc<dyn SharedElementStockPopTarget>,
    ) -> Rc<Self> {
        Rc::new(Self {
            navigation_controller: Rc::downgrade(navigation_controller),
            private_target: Rc::downgrade(private_target),
            latched_mode: Cell::new(SharedElementPopPanMode::Stock),
            active_mode: Cell::new(SharedElementPopPanMode::Stock),
            right_to_left_pop: RefCell::new(None),
        })
    }

    pub fn latch(&self, mode: SharedElementPopPanMode) {
        self.latched_mode.set(mode);
    }

    pub fn handle(&self, pan: &Rc<dyn SharedElementPanGesture>, cx: &mut App) {
        let state = pan.state();
        if state == SharedElementPanState::Began {
            self.active_mode.set(self.begin_mode(pan, cx));
            let latched = self.latched_mode.get();
            let active = self.active_mode.get();
            Sai::log(
                "SharedElement",
                move || format!("back pan began latched={latched:?} active={active:?}"),
                SaiLogKind::Ui,
                cx,
            );
        }

        match self.active_mode.get() {
            SharedElementPopPanMode::Stock | SharedElementPopPanMode::StockArmed => {
                if state == SharedElementPanState::Began
                    && NavigationInteractivePopGuard::is_right_to_left(cx)
                    && let Some(navigation_controller) = self.navigation_controller.upgrade()
                {
                    let pop = SharedElementNavigationDriver::shared().begin_right_to_left_pop(&navigation_controller, cx);
                    *self.right_to_left_pop.borrow_mut() = Some(pop);
                }
                let right_to_left_pop = self.right_to_left_pop.borrow().clone();
                if let Some(right_to_left_pop) = right_to_left_pop {
                    right_to_left_pop.handle(pan.as_ref(), cx);
                } else if let Some(private_target) = self.private_target.upgrade() {
                    private_target.handle_navigation_transition(pan, cx);
                }
            }
            SharedElementPopPanMode::Flight | SharedElementPopPanMode::Crossfade => {
                if state != SharedElementPanState::Began {
                    SharedElementNavigationDriver::shared().handle_interactive_pan(pan, cx);
                }
            }
            SharedElementPopPanMode::Ignore => {}
        }

        if matches!(
            state,
            SharedElementPanState::Ended | SharedElementPanState::Cancelled | SharedElementPanState::Failed
        ) {
            self.latched_mode.set(SharedElementPopPanMode::Stock);
            self.active_mode.set(SharedElementPopPanMode::Stock);
            *self.right_to_left_pop.borrow_mut() = None;
        }
    }

    fn begin_mode(&self, pan: &Rc<dyn SharedElementPanGesture>, cx: &mut App) -> SharedElementPopPanMode {
        let mode = self.latched_mode.get();
        if mode == SharedElementPopPanMode::Stock {
            return SharedElementPopPanMode::Stock;
        }
        let Some(navigation_controller) = self.navigation_controller.upgrade() else {
            return SharedElementPopPanMode::Stock;
        };
        SharedElementNavigationDriver::shared().begin_interactive_pop(
            &navigation_controller,
            pan,
            mode,
            SharedElementGesturePolicyAxis::Horizontal,
            cx,
        )
    }
}
