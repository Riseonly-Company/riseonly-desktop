use std::cell::RefCell;
use std::rc::Rc;

use gpui::{App, Pixels, Point, ScrollWheelEvent, Window};

pub struct PagerBackSwipeNavigationHooks {
    pub is_media_editor_editing: Rc<dyn Fn(&App) -> bool>,
    pub blocks_interactive_pop: Rc<dyn Fn(Point<Pixels>, &Window, &App) -> bool>,
    pub can_pop: Rc<dyn Fn(&Window, &App) -> bool>,
    pub is_right_to_left: Rc<dyn Fn(&App) -> bool>,
    pub drive_pop: Rc<dyn Fn(&ScrollWheelEvent, &mut Window, &mut App)>,
}

thread_local! {
    static PAGER_BACK_SWIPE_NAVIGATION_HOOKS: RefCell<Option<Rc<PagerBackSwipeNavigationHooks>>> =
        const { RefCell::new(None) };
}

/// installs the hooks, a later install replaces the earlier one
pub fn set_pager_back_swipe_navigation_hooks(hooks: PagerBackSwipeNavigationHooks) {
    PAGER_BACK_SWIPE_NAVIGATION_HOOKS.with(|slot| *slot.borrow_mut() = Some(Rc::new(hooks)));
}

impl PagerBackSwipeNavigationHooks {
    pub fn installed(_cx: &App) -> Option<Rc<PagerBackSwipeNavigationHooks>> {
        PAGER_BACK_SWIPE_NAVIGATION_HOOKS.with(|slot| slot.borrow().clone())
    }
}
