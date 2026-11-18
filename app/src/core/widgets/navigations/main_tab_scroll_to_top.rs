use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use gpui::{AnyWindowHandle, App, Bounds, Pixels, Point, ScrollWheelEvent, Window, point, px};
use rise_ui::guide_anchor_ui::guide_scroll_host::GuideScrollHost;

use crate::app::router::navigation_types::MainTab;
use crate::core::widgets::notifier::notifier_host::NotifierWindowPresenter;
use crate::modules::search::stores::search_interactions::search_interactions::SearchInteractionsStore;
use crate::modules::shorts::stores::shorts::shorts_interactions::shorts_interactions::ShortsInteractionsStore;

thread_local! {
    static LIVE_LISTS: RefCell<Vec<Weak<MainTabScrollToTopList>>> = const { RefCell::new(Vec::new()) };
    static ROOT_LIST_COLUMN: Cell<Option<(AnyWindowHandle, Bounds<Pixels>)>> = const { Cell::new(None) };
}

pub struct MainTabScrollToTop;

impl MainTabScrollToTop {
    const PROBE_HEIGHTS: [f32; 3] = [0.5, 0.35, 0.65];

    /// the open tab tapped again
    /// posts feed goes to top and refreshes, riffs goes back to the first riff with a fresh feed
    /// search goes to top and focuses its field
    pub fn reselect(tab: MainTab, window: &mut Window, cx: &mut App) {
        match tab {
            MainTab::Shorts => {
                ShortsInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.tab_reselected(cx));
            }
            MainTab::Posts => {
                Self::perform(true, window, cx);
            }
            MainTab::GlobalSearch => {
                Self::perform(false, window, cx);
                SearchInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.tab_reselected(cx));
            }
            _ => {
                Self::perform(false, window, cx);
            }
        }
    }

    /// true when a list engine view took the request (a feed refreshes there)
    /// plain scroll view or nothing on screen leaves the caller's own action to run
    pub fn perform(refreshes_feed: bool, window: &mut Window, cx: &mut App) -> bool {
        let Some(target) = Self::visible_list(window, cx) else {
            return false;
        };
        match target {
            MainTabScrollToTopTarget::List(list) => {
                (list.scroll_to_top)(refreshes_feed, window, cx);
                true
            }
            MainTabScrollToTopTarget::Scroll(scroll_view) => {
                let offset = scroll_view.content_offset();
                if offset.y > px(0.5) {
                    scroll_view.set_content_offset(point(offset.x, px(0.0)), true, cx);
                    window.refresh();
                }
                false
            }
        }
    }

    /// list column of the chat split holds the root page, center shows the open chat or placeholder
    /// none when the root moved back to the center
    pub fn set_root_list_column(bounds: Option<Bounds<Pixels>>, window: &Window) {
        let handle = window.window_handle();
        ROOT_LIST_COLUMN.with(|column| match bounds {
            Some(bounds) => column.set(Some((handle, bounds))),
            None => {
                if column.get().is_some_and(|(owner, _)| owner == handle) {
                    column.set(None);
                }
            }
        });
    }

    pub fn forget_frames(window: &Window) {
        let handle = window.window_handle();
        for scroll_view in GuideScrollHost::live() {
            if scroll_view.window() == Some(handle) {
                scroll_view.forget_frame();
            }
        }
        for list in MainTabScrollToTopList::live() {
            if list.frame_in(handle).is_some() {
                list.frame.set(None);
            }
        }
    }

    /// wheel over the empty sides of a width capped page scrolls the vertical list at that point
    /// as if the pointer was over it, true when a list took the wheel
    pub fn forward_wheel(at: Point<Pixels>, event: &ScrollWheelEvent, window: &mut Window, cx: &mut App) -> bool {
        let handle = window.window_handle();
        let Some(target) = Self::target_at(at, handle, &GuideScrollHost::live(), &MainTabScrollToTopList::live())
        else {
            return false;
        };
        match target {
            MainTabScrollToTopTarget::List(list) => match &list.wheel {
                Some(wheel) => {
                    wheel(event, window, cx);
                    true
                }
                None => false,
            },
            MainTabScrollToTopTarget::Scroll(scroll_view) => {
                let delta = event.delta.pixel_delta(window.line_height());
                let offset = scroll_view.content_offset();
                let maximum = scroll_view.max_content_offset().y.max(px(0.0));
                let target_y = (offset.y - delta.y).max(px(0.0)).min(maximum);
                if target_y != offset.y {
                    scroll_view.set_content_offset(point(offset.x, target_y), false, cx);
                    window.refresh();
                }
                true
            }
        }
    }

    fn visible_list(window: &Window, cx: &mut App) -> Option<MainTabScrollToTopTarget> {
        let handle = window.window_handle();
        let has_area = |column: &Bounds<Pixels>| column.size.width > px(0.0) && column.size.height > px(0.0);
        let root_list = ROOT_LIST_COLUMN
            .with(Cell::get)
            .filter(|(owner, column)| *owner == handle && has_area(column))
            .map(|(_, column)| column);
        let centre = NotifierWindowPresenter::shared(cx)
            .read(cx)
            .centre_column()
            .filter(has_area)
            .unwrap_or_else(|| Bounds::new(point(px(0.0), px(0.0)), window.viewport_size()));
        let scroll_views = GuideScrollHost::live();
        let lists = MainTabScrollToTopList::live();
        root_list.into_iter().chain(std::iter::once(centre)).find_map(|column| {
            Self::PROBE_HEIGHTS.into_iter().find_map(|fraction| {
                let at = point(
                    column.origin.x + column.size.width / 2.0,
                    column.origin.y + column.size.height * fraction,
                );
                Self::target_at(at, handle, &scroll_views, &lists)
            })
        })
    }

    fn target_at(
        at: Point<Pixels>,
        handle: AnyWindowHandle,
        scroll_views: &[GuideScrollHost],
        lists: &[Rc<MainTabScrollToTopList>],
    ) -> Option<MainTabScrollToTopTarget> {
        {
            let scroll_view = scroll_views
                .iter()
                .filter(|scroll_view| scroll_view.window() == Some(handle) && Self::is_vertical_list(scroll_view, at))
                .filter_map(|scroll_view| scroll_view.frame().map(|frame| (frame, scroll_view.clone())))
                .min_by(|(a, _), (b, _)| Self::area(a).total_cmp(&Self::area(b)));
            let list = lists
                .iter()
                .filter_map(|list| list.frame_in(handle).map(|frame| (frame, list.clone())))
                .filter(|(frame, _)| frame.contains(&at))
                .min_by(|(a, _), (b, _)| Self::area(a).total_cmp(&Self::area(b)));
            let target = match (scroll_view, list) {
                (Some((scroll_frame, scroll_view)), Some((list_frame, list))) => {
                    if Self::area(&list_frame) <= Self::area(&scroll_frame) {
                        MainTabScrollToTopTarget::List(list)
                    } else {
                        MainTabScrollToTopTarget::Scroll(scroll_view)
                    }
                }
                (Some((_, scroll_view)), None) => MainTabScrollToTopTarget::Scroll(scroll_view),
                (None, Some((_, list))) => MainTabScrollToTopTarget::List(list),
                (None, None) => return None,
            };
            Some(target)
        }
    }

    fn is_vertical_list(scroll_view: &GuideScrollHost, at: Point<Pixels>) -> bool {
        let Some(frame) = scroll_view.frame() else {
            return false;
        };
        frame.contains(&at)
            && scroll_view.answers_for(at)
            && scroll_view.is_scroll_enabled()
            && scroll_view.max_content_offset().y > px(1.0)
    }

    fn area(frame: &Bounds<Pixels>) -> f32 {
        f32::from(frame.size.width) * f32::from(frame.size.height)
    }
}

enum MainTabScrollToTopTarget {
    Scroll(GuideScrollHost),
    List(Rc<MainTabScrollToTopList>),
}

/// list engine view the probe must reach although it doesnt scroll through a kit scroll view
pub struct MainTabScrollToTopList {
    frame: Cell<Option<(AnyWindowHandle, Bounds<Pixels>)>>,
    scroll_to_top: Box<dyn Fn(bool, &mut Window, &mut App)>,
    wheel: Option<Box<dyn Fn(&ScrollWheelEvent, &mut Window, &mut App)>>,
}

impl MainTabScrollToTopList {
    fn live() -> Vec<Rc<MainTabScrollToTopList>> {
        LIVE_LISTS.with(|lists| {
            let mut lists = lists.borrow_mut();
            lists.retain(|weak| weak.strong_count() > 0);
            lists.iter().filter_map(Weak::upgrade).collect()
        })
    }

    fn frame_in(&self, window: AnyWindowHandle) -> Option<Bounds<Pixels>> {
        self.frame.get().filter(|(handle, _)| *handle == window).map(|(_, frame)| frame)
    }
}

/// owning end kept in the list view state, dropping it takes the list out of the probe
pub struct MainTabScrollToTopListMount {
    list: Rc<MainTabScrollToTopList>,
}

impl MainTabScrollToTopListMount {
    /// scroll_to_top is what happens on a hit
    /// feed refreshes if asked or jumps to top by remounting, other list views just scroll to top
    pub fn new(scroll_to_top: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        Self::mount(Box::new(scroll_to_top), None)
    }

    pub fn with_wheel(
        scroll_to_top: impl Fn(bool, &mut Window, &mut App) + 'static,
        wheel: impl Fn(&ScrollWheelEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self::mount(Box::new(scroll_to_top), Some(Box::new(wheel)))
    }

    fn mount(
        scroll_to_top: Box<dyn Fn(bool, &mut Window, &mut App)>,
        wheel: Option<Box<dyn Fn(&ScrollWheelEvent, &mut Window, &mut App)>>,
    ) -> Self {
        let list = Rc::new(MainTabScrollToTopList { frame: Cell::new(None), scroll_to_top, wheel });
        LIVE_LISTS.with(|lists| lists.borrow_mut().push(Rc::downgrade(&list)));
        Self { list }
    }

    pub fn did_lay_out(&self, bounds: Bounds<Pixels>, window: &Window) {
        self.list.frame.set(Some((window.window_handle(), bounds)));
    }

    /// the list left the window without being dropped
    pub fn did_leave_window(&self) {
        self.list.frame.set(None);
    }
}
