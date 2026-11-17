use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui::{App, Bounds, Entity, Pixels, Point, WeakEntity, Window};

use crate::app::router::navigation_fullscreen_pop::NavigationFullscreenPopPan;
use crate::app::router::navigation_store::NavigationStore;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;
use crate::core::shared_element_transition::kernel::shared_element_types::{
    ProfilePostListWindowPolicy, SharedElementGesturePolicyAxis, SharedElementPopPanMode,
};
use crate::core::shared_element_transition::navigation::shared_element_navigation_driver::SharedElementNavigationDriver;
use crate::core::shared_element_transition::navigation::shared_element_navigation_transitioning::{
    SharedElementNavigationController, SharedElementPanGesture, SharedElementPanState,
};
use crate::core::shared_element_transition::runtime::shared_element_navigation_policy::SharedElementNavigationPolicy;
use crate::core::ui::bottom_sheet_ui::experimental_bottom_sheet_presentation_host::SheetLayerStack;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_types::RiseManualListScrollMetrics;
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_view::{
    RiseManualListScrollGestureRecognizing, RiseManualListView,
};

const LOG_NAME: &str = "SharedElement";

pub type PostListPullDownDismissAllowsBegin = Rc<dyn Fn(&App) -> bool>;
pub type PostListPullDownDismissNavigationController = Rc<dyn Fn(&App) -> Option<Entity<NavigationStore>>>;

pub struct PostListPullDownDismiss {
    list_view: WeakEntity<RiseManualListView>,
    recognizer: Rc<RefCell<dyn RiseManualListScrollGestureRecognizing>>,
    shared: Rc<PostListPullDownDismissShared>,
    is_installed: bool,
}

struct PostListPullDownDismissShared {
    allows_begin: RefCell<PostListPullDownDismissAllowsBegin>,
    navigation_controller: RefCell<PostListPullDownDismissNavigationController>,
    is_dismissing: Cell<bool>,
    distance_from_top: Cell<f32>,
    is_decelerating: Cell<bool>,
}

struct PostListPullDownDismissRecognizer {
    shared: Rc<PostListPullDownDismissShared>,
    pan: Option<Rc<NavigationFullscreenPopPan>>,
    last_position: Point<Pixels>,
    began: bool,
    dismiss_stack: Option<Rc<dyn SharedElementNavigationController>>,
    dismiss_mode: SharedElementPopPanMode,
}

impl PostListPullDownDismiss {
    pub const RECOGNIZER_NAME: &'static str = "riseonly.postListPullDownDismiss";

    pub fn new(list_view: WeakEntity<RiseManualListView>) -> Self {
        let shared = Rc::new(PostListPullDownDismissShared {
            allows_begin: RefCell::new(Rc::new(|_: &App| true)),
            navigation_controller: RefCell::new(Rc::new(|_: &App| None)),
            is_dismissing: Cell::new(false),
            distance_from_top: Cell::new(0.0),
            is_decelerating: Cell::new(false),
        });
        let recognizer: Rc<RefCell<dyn RiseManualListScrollGestureRecognizing>> =
            Rc::new(RefCell::new(PostListPullDownDismissRecognizer {
                shared: shared.clone(),
                pan: None,
                last_position: Point::default(),
                began: false,
                dismiss_stack: None,
                dismiss_mode: SharedElementPopPanMode::Ignore,
            }));
        Self { list_view, recognizer, shared, is_installed: false }
    }

    pub fn set_allows_begin(&self, allows_begin: impl Fn(&App) -> bool + 'static) {
        *self.shared.allows_begin.borrow_mut() = Rc::new(allows_begin);
    }

    pub fn set_navigation_controller(&self, navigation_controller: impl Fn(&App) -> Option<Entity<NavigationStore>> + 'static) {
        *self.shared.navigation_controller.borrow_mut() = Rc::new(navigation_controller);
    }

    pub fn is_dismissing(&self) -> bool {
        self.shared.is_dismissing.get()
    }

    /// every metrics change, next to the refresh control observe
    pub fn scroll_metrics_did_change(&self, metrics: &RiseManualListScrollMetrics) {
        self.shared.distance_from_top.set(metrics.distance_from_top.as_f32());
        self.shared.is_decelerating.set(metrics.is_decelerating);
    }

    pub fn install(&mut self, cx: &mut App) {
        if self.is_installed {
            return;
        }
        let recognizer = self.recognizer.clone();
        if self.list_view.update(cx, |list, _| list.add_scroll_gesture_recognizer(recognizer, true)).is_ok() {
            self.is_installed = true;
        }
    }

    pub fn stop(&mut self, cx: &mut App) {
        if self.is_installed {
            let recognizer = self.recognizer.clone();
            let _ = self.list_view.update(cx, |list, _| list.remove_scroll_gesture_recognizer(&recognizer));
            self.is_installed = false;
        }
        self.shared.is_dismissing.set(false);
        *self.shared.allows_begin.borrow_mut() = Rc::new(|_: &App| false);
        *self.shared.navigation_controller.borrow_mut() = Rc::new(|_: &App| None);
    }

    /// esc or the back button over the list, the exit without a drag
    /// stack delegate picks the flight for the pop
    /// false when nothing closed so esc can fall through
    pub fn request_dismiss(&self, cx: &mut App) -> bool {
        if self.shared.is_dismissing.get() || HoldContextMenuPresenter::shared(cx).read(cx).active_request().is_some() {
            return false;
        }
        let navigation_controller = self.shared.navigation_controller.borrow().clone();
        let Some(navigation_controller) = navigation_controller(cx) else { return false };
        if PostListPullDownDismissShared::usable_stack(&navigation_controller, cx).is_none() {
            return false;
        }
        navigation_controller.update(cx, |store, cx| store.pop(cx));
        true
    }
}

impl PostListPullDownDismissShared {
    /// more than one screen, nothing presented over the stack and no transition running
    fn usable_stack(
        navigation_controller: &Entity<NavigationStore>,
        cx: &App,
    ) -> Option<Rc<dyn SharedElementNavigationController>> {
        let store = navigation_controller.read(cx);
        if store.path.is_empty() || !SheetLayerStack::top_is_root(cx) || store.bot_mini_app_presentation().is_some() {
            return None;
        }
        let stack = store.navigation_window_stack()?;
        if stack.transition_coordinator(cx).is_some() {
            return None;
        }
        Some(stack)
    }

    fn base_conditions_hold(&self, cx: &mut App) -> bool {
        if self.is_dismissing.get()
            || !SharedElementNavigationPolicy::flights_enabled()
            || !SharedElementNavigationPolicy::interactive_enabled()
            || HoldContextMenuPresenter::shared(cx).read(cx).active_request().is_some()
        {
            return false;
        }
        let allows_begin = self.allows_begin.borrow().clone();
        allows_begin(cx) && self.distance_from_top.get() <= ProfilePostListWindowPolicy::PULL_DOWN_TOP_TOLERANCE
    }
}

impl PostListPullDownDismissRecognizer {
    fn can_receive_touch(&self, cx: &mut App) -> bool {
        !self.shared.is_decelerating.get() && self.shared.base_conditions_hold(cx)
    }

    fn should_begin(&mut self, pan: &Rc<NavigationFullscreenPopPan>, cx: &mut App) -> bool {
        if !self.shared.base_conditions_hold(cx) {
            return false;
        }
        let velocity = pan.velocity();
        if velocity.y <= Pixels::ZERO || velocity.y <= velocity.x.abs() {
            return false;
        }
        let navigation_controller = self.shared.navigation_controller.borrow().clone();
        let Some(navigation_controller) = navigation_controller(cx) else { return false };
        let Some(stack) = PostListPullDownDismissShared::usable_stack(&navigation_controller, cx) else {
            return false;
        };
        let gesture: Rc<dyn SharedElementPanGesture> = pan.clone();
        let mode = SharedElementNavigationDriver::shared().pop_pan_mode(&stack, &gesture, cx);
        if mode != SharedElementPopPanMode::Flight && mode != SharedElementPopPanMode::Crossfade {
            return false;
        }
        self.dismiss_stack = Some(stack);
        self.dismiss_mode = mode;
        true
    }

    fn handle_began(&mut self, pan: &Rc<NavigationFullscreenPopPan>, cx: &mut App) {
        let Some(stack) = self.dismiss_stack.clone() else { return };
        pan.set_state(SharedElementPanState::Began);
        let gesture: Rc<dyn SharedElementPanGesture> = pan.clone();
        let mode = SharedElementNavigationDriver::shared().begin_interactive_pop(
            &stack,
            &gesture,
            self.dismiss_mode,
            SharedElementGesturePolicyAxis::Vertical,
            cx,
        );
        let line = format!("native pull-down began mode={mode:?}");
        Sai::log(LOG_NAME, move || line, SaiLogKind::Ui, cx);
        if mode == SharedElementPopPanMode::StockArmed || mode == SharedElementPopPanMode::Stock {
            stack.pop_view_controller(true, cx);
            self.reset();
            return;
        }
        self.shared
            .is_dismissing
            .set(mode == SharedElementPopPanMode::Flight || mode == SharedElementPopPanMode::Crossfade);
    }

    fn reset(&mut self) {
        self.pan = None;
        self.began = false;
        self.dismiss_stack = None;
        self.dismiss_mode = SharedElementPopPanMode::Ignore;
    }
}

impl RiseManualListScrollGestureRecognizing for PostListPullDownDismissRecognizer {
    fn pointer_down(&mut self, position: Point<Pixels>, window: &mut Window, cx: &mut App) -> bool {
        self.reset();
        if !self.can_receive_touch(cx) {
            return false;
        }
        let view_bounds = Some(Bounds::new(Point::default(), window.viewport_size()));
        self.last_position = position;
        self.pan = Some(Rc::new(NavigationFullscreenPopPan::new(position, view_bounds)));
        true
    }

    fn pointer_moved(&mut self, position: Point<Pixels>, _window: &mut Window, cx: &mut App) -> bool {
        let Some(pan) = self.pan.clone() else { return false };
        pan.add_translation(position - self.last_position, position);
        self.last_position = position;
        if !self.began {
            if !self.should_begin(&pan, cx) {
                pan.set_state(SharedElementPanState::Failed);
                self.reset();
                return false;
            }
            self.began = true;
            self.handle_began(&pan, cx);
            return self.pan.is_some();
        }
        if self.shared.is_dismissing.get() {
            pan.set_state(SharedElementPanState::Changed);
            let gesture: Rc<dyn SharedElementPanGesture> = pan;
            SharedElementNavigationDriver::shared().handle_interactive_pan(&gesture, cx);
        }
        true
    }

    fn pointer_up(&mut self, position: Point<Pixels>, _window: &mut Window, cx: &mut App) {
        let Some(pan) = self.pan.clone() else { return };
        pan.add_translation(position - self.last_position, position);
        if self.shared.is_dismissing.get() {
            pan.set_state(SharedElementPanState::Ended);
            let gesture: Rc<dyn SharedElementPanGesture> = pan;
            SharedElementNavigationDriver::shared().handle_interactive_pan(&gesture, cx);
        }
        self.shared.is_dismissing.set(false);
        self.reset();
    }
}
