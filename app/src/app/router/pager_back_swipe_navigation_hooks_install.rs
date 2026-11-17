use std::rc::Rc;

use gpui::{App, Pixels, Point, ScrollWheelEvent, Window};
use rise_ui::pager_back_swipe_navigation_hooks::{PagerBackSwipeNavigationHooks, set_pager_back_swipe_navigation_hooks};

use crate::app::router::navigation_fullscreen_pop::{
    NavigationFullscreenPop, NavigationFullscreenPopDelegate, NavigationInteractivePopGuard,
};

pub fn install_pager_back_swipe_navigation_hooks() {
    set_pager_back_swipe_navigation_hooks(PagerBackSwipeNavigationHooks {
        is_media_editor_editing: Rc::new(NavigationInteractivePopGuard::is_media_editor_editing),
        blocks_interactive_pop: Rc::new(blocks_interactive_pop),
        can_pop: Rc::new(can_pop),
        is_right_to_left: Rc::new(NavigationInteractivePopGuard::is_right_to_left),
        drive_pop: Rc::new(drive_pop),
    });
}

fn blocks_interactive_pop(at_window_point: Point<Pixels>, _window: &Window, cx: &App) -> bool {
    NavigationInteractivePopGuard::blocks_interactive_pop(at_window_point, cx)
}

/// true if the stack can pop now, more than one screen and nothing presented over it
fn can_pop(_window: &Window, cx: &App) -> bool {
    NavigationFullscreenPop::active_delegate(cx).is_some_and(|delegate| delegate.can_pop(cx))
}

fn drive_pop(event: &ScrollWheelEvent, window: &mut Window, cx: &mut App) {
    let delegate: Option<Rc<NavigationFullscreenPopDelegate>> = NavigationFullscreenPop::active_delegate(cx);
    if let Some(delegate) = delegate {
        delegate.drive_pop(event, window, cx);
    }
}
