use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, App, Context, ElementId, Entity, InteractiveElement, IntoElement, KeyDownEvent, List, ListAlignment,
    ListOffset, ListScrollEvent, ListState, ParentElement, RenderOnce, Styled, Task, Window, div, list, px,
};
use rise_theme::AppTheme;

use crate::guide_anchor_ui::guide_scroll_host::{GuideScrollHostMount, GuideScrollHostScope};
use crate::swiftui::lifecycle::LifecycleModifiers;
use crate::swiftui::page_refresh::{PageFocus, is_page_refresh_keystroke};
use crate::swiftui::scroll_view::{RefreshAction, RefreshPull, refresh_header, scroll_content_bottom_margin};

/// long list over the gpui one
/// a row asks for the next page itself via sai_pagination_prefetch
/// rows need a stable server id, gpui keys by index
pub struct ListUi {
    state: ListState,
}

impl ListUi {
    pub fn new(theme: &AppTheme, item_count: usize, alignment: ListAlignment) -> Self {
        Self {
            state: ListState::new(item_count, alignment, theme.list.overdraw),
        }
    }

    pub fn state(&self) -> &ListState {
        &self.state
    }

    pub fn item_count(&self) -> usize {
        self.state.item_count()
    }

    /// rows arrived at the end, viewport does not move
    pub fn appended(&self, count: usize) {
        if count == 0 {
            return;
        }
        let end = self.state.item_count();
        self.state.splice(end..end, count);
    }

    /// rows arrived at the start, the row in view stays where it is
    pub fn prepended(&self, count: usize) {
        if count == 0 {
            return;
        }
        self.state.splice(0..0, count);
    }

    pub fn removed(&self, range: Range<usize>) {
        if range.is_empty() {
            return;
        }
        self.state.splice(range, 0);
    }

    /// a different list entirely, unlike the splice calls this moves the viewport to 0
    pub fn replaced(&self, count: usize) {
        self.state.reset(count);
    }

    /// same list rebuilt, a refresh reordered or trimmed rows
    /// viewport stays put, the old top row keeps its offset if new_index_of finds it
    pub fn rebuilt(&self, count: usize, new_index_of: impl Fn(usize) -> Option<usize>) {
        let top = self.state.logical_scroll_top();
        self.state.reset(count);
        if top.item_ix == 0 && top.offset_in_item <= px(0.) {
            return;
        }
        let scroll_top = match new_index_of(top.item_ix) {
            Some(item_ix) => ListOffset { item_ix, offset_in_item: top.offset_in_item },
            None => ListOffset { item_ix: top.item_ix.min(count.saturating_sub(1)), offset_in_item: px(0.) },
        };
        self.state.scroll_to(scroll_top);
    }

    pub fn on_visible_range<V: 'static>(
        &self,
        view: &Entity<V>,
        reported: impl Fn(&mut V, Range<usize>, usize, &mut Context<V>) + 'static,
    ) {
        let view = view.downgrade();
        let reported = Rc::new(reported);
        let latest: Rc<RefCell<Option<(Range<usize>, usize)>>> = Rc::new(RefCell::new(None));

        self.state.set_scroll_handler(move |event: &ListScrollEvent, _window, cx| {
            let already_scheduled = latest
                .borrow_mut()
                .replace((event.visible_range.clone(), event.count))
                .is_some();
            if already_scheduled {
                return;
            }

            let view = view.clone();
            let reported = Rc::clone(&reported);
            let latest = Rc::clone(&latest);

            cx.defer(move |cx| {
                let Some((range, count)) = latest.borrow_mut().take() else {
                    return;
                };
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |view, cx| reported(view, range, count, cx));
            });
        });
    }

    /// list fills its container
    /// gpui asks taffy for height auto and the list keeps rows out of layout so an unsized list
    /// paints nothing
    pub fn element(
        &self,
        render_item: impl FnMut(usize, &mut Window, &mut App) -> AnyElement + 'static,
    ) -> List {
        list(self.state.clone(), render_item).size_full()
    }

    /// element with pull to refresh, trackpad pull past the top edge
    /// a pull while the last refresh still runs starts none
    pub fn refreshable(
        &self,
        id: impl Into<ElementId>,
        action: impl Fn(&mut Window, &mut App) -> Task<()> + 'static,
        render_item: impl FnMut(usize, &mut Window, &mut App) -> AnyElement + 'static,
    ) -> RefreshableListUi {
        RefreshableListUi {
            id: id.into(),
            state: self.state.clone(),
            list: self.element(render_item),
            action: Rc::new(action),
        }
    }

    /// element for a lazy scroll view, rows built lazily
    /// list answers as a GuideScrollHost so a guide or gate walking up finds the enclosing scroll view
    pub fn hosted(
        &self,
        id: impl Into<ElementId>,
        render_item: impl FnMut(usize, &mut Window, &mut App) -> AnyElement + 'static,
    ) -> HostedListUi {
        HostedListUi {
            id: id.into(),
            state: self.state.clone(),
            list: self.element(render_item),
            scroll_disabled: false,
            background: None,
        }
    }
}

#[derive(IntoElement)]
pub struct HostedListUi {
    id: ElementId,
    state: ListState,
    list: List,
    scroll_disabled: bool,
    background: Option<AnyElement>,
}

impl HostedListUi {
    /// turns scrolling off, rows stay tappable
    pub fn scroll_disabled(mut self, disabled: bool) -> Self {
        self.scroll_disabled = disabled;
        self
    }

    /// background of the scrolled content, drawn under the rows inside the host
    /// guideScrollHost current names this list while it prepaints
    pub fn background(mut self, background: impl IntoElement) -> Self {
        self.background = Some(background.into_any_element());
        self
    }
}

impl RenderOnce for HostedListUi {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let list_state = self.state;
        let mount = window.use_keyed_state(self.id, cx, move |_, _| GuideScrollHostMount::for_list(list_state));
        let host = mount.read(cx).host();
        host.set_scroll_enabled(!self.scroll_disabled);

        let wheel_host = host.clone();
        let wheel_lock = self
            .scroll_disabled
            .then(|| div().absolute().inset_0().on_scroll_wheel(|_, _, cx| cx.stop_propagation()));
        let content = div()
            .relative()
            .size_full()
            .on_scroll_wheel(move |event, _, _| wheel_host.note_wheel(event))
            .children(self.background)
            .child(self.list.pb(px(scroll_content_bottom_margin(cx))))
            .children(wheel_lock);
        GuideScrollHostScope::new(host, content)
    }
}

const TOP_TOLERANCE: f32 = 0.5;

fn scrolled_from_top(state: &ListState) -> f32 {
    (-f32::from(state.scroll_px_offset_for_scrollbar().y)).max(0.0)
}

/// what the refreshable list keeps between frames
struct RefreshableListState {
    refresh: RefreshPull,
    scrolled: f32,
}

#[derive(IntoElement)]
pub struct RefreshableListUi {
    id: ElementId,
    state: ListState,
    list: List,
    action: RefreshAction,
}

impl RenderOnce for RefreshableListUi {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.id.clone(), cx, |_, _| RefreshableListState {
            refresh: RefreshPull::new(),
            scrolled: 0.0,
        });
        let list_state = self.state;
        let scrolled = scrolled_from_top(&list_state);
        let viewport = f32::from(list_state.viewport_bounds().size.height);
        let (shown_pull, refreshing) = state.update(cx, |state, _| {
            state.scrolled = scrolled;
            (state.refresh.shown_pull(viewport), state.refresh.is_refreshing())
        });
        let shown_pull = (shown_pull - scrolled).max(0.0);
        let header = (shown_pull > 0.0).then(|| refresh_header(shown_pull, refreshing));
        let list = self.list.pb(px(scroll_content_bottom_margin(cx)));
        let content = div().absolute().left_0().right_0().top(px(shown_pull)).bottom(px(-shown_pull)).child(list);

        let action = self.action;
        let focus_id = ElementId::NamedChild(Arc::new(self.id.clone()), "refresh-focus".into());
        let appear_id = ElementId::NamedChild(Arc::new(self.id.clone()), "refresh-focus-appear".into());
        let page_focus = PageFocus::keyed(focus_id, window, cx);
        let focus = page_focus.read(cx).handle.clone();
        let leaving = page_focus.clone();
        let key_state = state.clone();
        let key_action = action.clone();
        div()
            .id(self.id)
            .track_focus(&focus)
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                if !is_page_refresh_keystroke(&event.keystroke) {
                    return;
                }
                cx.stop_propagation();
                if key_state.read(cx).refresh.is_refreshing() {
                    return;
                }
                let work = key_action(window, cx);
                key_state.update(cx, |state, cx| {
                    state.refresh.begin_refresh(work, |state: &mut RefreshableListState| &mut state.refresh, cx);
                });
            })
            .relative()
            .size_full()
            .overflow_hidden()
            .on_scroll_wheel(move |event, window, cx| {
                let delta = f32::from(event.delta.pixel_delta(window.line_height()).y);
                let scrolled = scrolled_from_top(&list_state);
                let dimension = f32::from(list_state.viewport_bounds().size.height);
                let step = state.update(cx, |state, cx| {
                    let overscroll =
                        if scrolled <= TOP_TOLERANCE { (delta - state.scrolled).max(0.0) } else { 0.0 };
                    let pull = state.refresh.pull();
                    let step = state.refresh.track_pull(event, delta, overscroll, dimension);
                    state.scrolled = (scrolled - step.absorbed).max(0.0);
                    state.refresh.arm_release(|state: &mut RefreshableListState| &mut state.refresh, cx);
                    if state.refresh.pull() != pull {
                        cx.notify();
                    }
                    step
                });
                if step.absorbed > 0.0 {
                    list_state.scroll_by(px(-step.absorbed));
                }
                if step.starts {
                    let work = action(window, cx);
                    state.update(cx, |state, cx| {
                        state.refresh.begin_refresh(work, |state: &mut RefreshableListState| &mut state.refresh, cx);
                    });
                }
            })
            .children(header)
            .child(content)
            .on_appear(appear_id, move |window, cx| PageFocus::did_appear(&page_focus, window, cx))
            .and_on_disappear(move |window, cx| {
                if let Some(window) = window {
                    PageFocus::will_disappear(&leaving, window, cx);
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use futures::channel::oneshot;
    use gpui::{Modifiers, ScrollDelta, ScrollWheelEvent, TestAppContext, TouchPhase, VisualTestContext, point};

    use super::*;
    use crate::guide_anchor_ui::guide_scroll_host::GuideScrollHost;

    const ROW: f32 = 100.0;

    struct Probe {
        build: Box<dyn Fn() -> AnyElement>,
    }

    impl gpui::Render for Probe {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().w(px(200.)).h(px(ROW)).child((self.build)())
        }
    }

    fn open_refreshable(
        cx: &mut TestAppContext,
        action: impl Fn(&mut Window, &mut App) -> Task<()> + Clone + 'static,
    ) -> (Rc<ListUi>, &mut VisualTestContext) {
        let theme = AppTheme::dark();
        cx.update(|cx| crate::install_theme(theme.clone(), cx));
        let list = Rc::new(cx.update(|_| ListUi::new(&theme, 6, ListAlignment::Top)));
        let shown = list.clone();
        let (_, cx) = cx.add_window_view(move |_, _| Probe {
            build: {
                let shown = shown.clone();
                let action = action.clone();
                Box::new(move || {
                    shown
                        .refreshable("refreshable_list", action.clone(), |_, _, _| {
                            div().w_full().h(px(ROW)).into_any_element()
                        })
                        .into_any_element()
                })
            },
        });
        cx.run_until_parked();
        (list, cx)
    }

    fn wheel(cx: &mut VisualTestContext, dy: f32, phase: TouchPhase) {
        cx.simulate_event(ScrollWheelEvent {
            position: point(px(10.), px(10.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(dy))),
            modifiers: Modifiers::none(),
            touch_phase: phase,
        });
        cx.run_until_parked();
    }

    fn pull_down(cx: &mut VisualTestContext) {
        wheel(cx, 0.0, TouchPhase::Started);
        for _ in 0..10 {
            wheel(cx, 50.0, TouchPhase::Moved);
        }
        wheel(cx, 0.0, TouchPhase::Ended);
    }

    #[gpui::test]
    fn a_pull_past_the_top_starts_one_refresh_per_gesture(cx: &mut TestAppContext) {
        let calls = Rc::new(Cell::new(0));
        let counted = calls.clone();
        let (_list, cx) = open_refreshable(cx, move |_, _| {
            counted.set(counted.get() + 1);
            Task::ready(())
        });

        pull_down(cx);
        assert_eq!(calls.get(), 1);

        pull_down(cx);
        assert_eq!(calls.get(), 2);
    }

    #[gpui::test]
    fn the_momentum_after_a_lift_starts_no_refresh(cx: &mut TestAppContext) {
        let calls = Rc::new(Cell::new(0));
        let counted = calls.clone();
        let (list, cx) = open_refreshable(cx, move |_, _| {
            counted.set(counted.get() + 1);
            Task::ready(())
        });

        wheel(cx, 0.0, TouchPhase::Started);
        wheel(cx, 0.0, TouchPhase::Ended);
        for _ in 0..10 {
            wheel(cx, 50.0, TouchPhase::Moved);
        }
        assert_eq!(calls.get(), 0);
        assert_eq!(scrolled_from_top(list.state()), 0.0);
    }

    #[gpui::test]
    fn a_step_that_only_brings_the_list_back_to_the_top_is_no_pull(cx: &mut TestAppContext) {
        let calls = Rc::new(Cell::new(0));
        let counted = calls.clone();
        let (list, cx) = open_refreshable(cx, move |_, _| {
            counted.set(counted.get() + 1);
            Task::ready(())
        });

        wheel(cx, -150.0, TouchPhase::Moved);
        assert_eq!(scrolled_from_top(list.state()), 150.0);

        wheel(cx, 0.0, TouchPhase::Started);
        wheel(cx, 150.0, TouchPhase::Moved);
        assert_eq!(scrolled_from_top(list.state()), 0.0);
        assert_eq!(calls.get(), 0);
    }

    #[gpui::test]
    fn a_pull_while_refreshing_does_not_start_a_second_refresh(cx: &mut TestAppContext) {
        let pending: Rc<RefCell<Vec<oneshot::Sender<()>>>> = Rc::default();
        let calls = pending.clone();
        let (_list, cx) = open_refreshable(cx, move |_, cx| {
            let (finish, finished) = oneshot::channel::<()>();
            calls.borrow_mut().push(finish);
            cx.background_executor().spawn(async move {
                finished.await.ok();
            })
        });

        pull_down(cx);
        assert_eq!(pending.borrow().len(), 1);

        pull_down(cx);
        assert_eq!(pending.borrow().len(), 1);

        let first = pending.borrow_mut().remove(0);
        first.send(()).ok();
        cx.run_until_parked();

        pull_down(cx);
        assert_eq!(pending.borrow().len(), 1);
    }

    fn open_hosted(cx: &mut TestAppContext, scroll_disabled: bool) -> &mut VisualTestContext {
        let theme = AppTheme::dark();
        cx.update(|cx| crate::install_theme(theme.clone(), cx));
        let list = Rc::new(cx.update(|_| ListUi::new(&theme, 6, ListAlignment::Top)));
        let (_, cx) = cx.add_window_view(move |_, _| Probe {
            build: {
                let list = list.clone();
                Box::new(move || {
                    list.hosted("hosted_list", |_, _, _| div().w_full().h(px(ROW)).into_any_element())
                        .scroll_disabled(scroll_disabled)
                        .into_any_element()
                })
            },
        });
        cx.run_until_parked();
        cx
    }

    fn only_live_host() -> GuideScrollHost {
        let mut hosts = GuideScrollHost::live();
        assert_eq!(hosts.len(), 1);
        hosts.remove(0)
    }

    #[gpui::test]
    fn a_hosted_list_answers_for_its_offset_as_a_scroll_view_does(cx: &mut TestAppContext) {
        let cx = open_hosted(cx, false);
        let host = only_live_host();
        assert_eq!(host.content_offset().y, px(0.));

        wheel(cx, -150.0, TouchPhase::Moved);
        assert_eq!(host.content_offset().y, px(150.));
        assert!(host.max_content_offset().y >= px(150.));

        cx.update(|_, cx| host.set_content_offset(point(px(0.), px(50.)), false, cx));
        assert_eq!(host.content_offset().y, px(50.));
    }

    #[gpui::test]
    fn a_disabled_hosted_list_keeps_its_offset_under_the_wheel(cx: &mut TestAppContext) {
        let cx = open_hosted(cx, true);
        let host = only_live_host();
        assert!(!host.is_scroll_enabled());

        wheel(cx, -150.0, TouchPhase::Moved);
        assert_eq!(host.content_offset().y, px(0.));
    }

    #[gpui::test]
    fn splicing_at_an_end_keeps_the_rows_that_were_already_there(cx: &mut gpui::TestAppContext) {
        let theme = AppTheme::dark();
        let list = cx.update(|_| ListUi::new(&theme, 20, ListAlignment::Top));

        list.appended(10);
        assert_eq!(list.item_count(), 30);

        list.prepended(5);
        assert_eq!(list.item_count(), 35);

        list.removed(0..5);
        assert_eq!(list.item_count(), 30);

        list.replaced(3);
        assert_eq!(list.item_count(), 3);
    }

    #[gpui::test]
    fn splicing_nothing_is_a_no_op_rather_than_a_reset(cx: &mut gpui::TestAppContext) {
        let theme = AppTheme::dark();
        let list = cx.update(|_| ListUi::new(&theme, 20, ListAlignment::Top));

        list.appended(0);
        list.prepended(0);
        list.removed(5..5);
        assert_eq!(list.item_count(), 20);
    }
}
