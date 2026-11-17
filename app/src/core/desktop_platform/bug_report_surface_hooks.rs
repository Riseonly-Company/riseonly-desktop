use std::cell::{Cell, RefCell};

use rise_canon::RiseBugReportSurface;
use rise_diagnostics::rise_breadcrumbs::{RiseBreadcrumbs, RiseBreadcrumbsChannel};

use crate::app::router::navigation_types::{MainTab, RootRoute};

thread_local! {
    static CURRENT: Cell<RiseBugReportSurface> = const { Cell::new(RiseBugReportSurface::Other) };
    static LAST_RECORDED_SCREEN: RefCell<String> = const { RefCell::new(String::new()) };
}

/// carries the current area to whoever opens a report with no view hierarchy of its own
/// written once per navigation and read once per opening, deliberately not observable
pub enum BugReportSurfaceProbe {}

impl BugReportSurfaceProbe {
    pub fn current() -> RiseBugReportSurface {
        CURRENT.get()
    }

    pub fn update(path: &[RootRoute], tab: MainTab) {
        let surface = path.last().map_or_else(|| tab.bug_report_surface(), RootRoute::bug_report_surface);
        CURRENT.set(surface);

        let screen = path.last().map_or_else(|| format!("{tab:?}"), Self::route_name);
        let is_new = LAST_RECORDED_SCREEN.with_borrow_mut(|last| {
            if *last == screen {
                return false;
            }
            last.clone_from(&screen);
            true
        });
        if is_new {
            RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Screen, || format!("opened {screen}"));
        }
    }

    fn route_name(route: &RootRoute) -> String {
        route.sai_active_screen().id
    }
}
