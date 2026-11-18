use crate::app::aside::aside_navigator;
use crate::app::router::navigation_aside_hooks::{NavigationAsideHooks, set_navigation_aside_hooks};

pub fn install_navigation_aside_hooks() {
    set_navigation_aside_hooks(NavigationAsideHooks {
        open: aside_navigator::open,
        open_over_default: aside_navigator::open_over_default,
        has_frontmost: aside_navigator::has_frontmost,
    });
}
