use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui::{
    AsyncWindowContext, Context, IntoElement, ParentElement, Render, TestAppContext,
    VisualTestContext, Window, WindowHandle, div,
};

use super::lifecycle::{LifecycleModifiers, on_change, use_task};

type Log = Rc<RefCell<Vec<&'static str>>>;

struct AppearHost {
    shown: bool,
    log: Log,
}

impl Render for AppearHost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let mut root = div();
        if self.shown {
            let appeared = self.log.clone();
            let disappeared = self.log.clone();
            root = root.child(
                div()
                    .on_appear("row", move |_, _| appeared.borrow_mut().push("appear"))
                    .and_on_disappear(move |_, _| disappeared.borrow_mut().push("disappear")),
            );
        }
        root
    }
}

fn open<V: Render>(cx: &mut TestAppContext, view: V) -> WindowHandle<V> {
    let window = cx.add_window(|_, _| view);
    VisualTestContext::from_window(window.into(), cx).run_until_parked();
    window
}

fn redraw<V: Render>(
    cx: &mut TestAppContext,
    window: WindowHandle<V>,
    change: impl FnOnce(&mut V),
) {
    let updated = window.update(cx, |view, _, cx| {
        change(view);
        cx.notify();
    });
    assert!(updated.is_ok(), "the test window is open");
    cx.run_until_parked();
}

#[gpui::test]
fn appear_runs_once_while_the_view_stays_drawn(cx: &mut TestAppContext) {
    let log: Log = Rc::default();
    let window = open(
        cx,
        AppearHost {
            shown: true,
            log: log.clone(),
        },
    );
    assert_eq!(*log.borrow(), ["appear"]);
    redraw(cx, window, |_| {});
    assert_eq!(*log.borrow(), ["appear"], "a later frame is not a new appearance");
}

struct ClosingHost {
    log: Log,
}

impl Render for ClosingHost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let log = self.log.clone();
        div().on_disappear("row", move |window, _| {
            log.borrow_mut().push(if window.is_some() { "drawn away" } else { "window closed" });
        })
    }
}

#[gpui::test]
fn disappear_runs_without_a_window_when_the_window_closes(cx: &mut TestAppContext) {
    let log: Log = Rc::default();
    let window = open(cx, ClosingHost { log: log.clone() });
    let closed = window.update(cx, |_, window, _| window.remove_window());
    assert!(closed.is_ok(), "the test window is open");
    cx.run_until_parked();
    assert_eq!(*log.borrow(), ["window closed"]);
}

#[gpui::test]
fn disappear_runs_when_the_view_stops_being_drawn(cx: &mut TestAppContext) {
    let log: Log = Rc::default();
    let window = open(
        cx,
        AppearHost {
            shown: true,
            log: log.clone(),
        },
    );
    redraw(cx, window, |host| host.shown = false);
    assert_eq!(*log.borrow(), ["appear", "disappear"]);
}

#[gpui::test]
fn drawing_again_after_a_gap_appears_again(cx: &mut TestAppContext) {
    let log: Log = Rc::default();
    let window = open(
        cx,
        AppearHost {
            shown: true,
            log: log.clone(),
        },
    );
    redraw(cx, window, |host| host.shown = false);
    redraw(cx, window, |host| host.shown = true);
    assert_eq!(*log.borrow(), ["appear", "disappear", "appear"]);
}

struct CancelCount(Rc<Cell<u32>>);

impl Drop for CancelCount {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

struct TaskHost {
    shown: bool,
    task_id: u32,
    started: Rc<Cell<u32>>,
    cancelled: Rc<Cell<u32>>,
}

impl Render for TaskHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.shown {
            let started = self.started.clone();
            let cancelled = self.cancelled.clone();
            use_task(
                "load",
                self.task_id,
                window,
                cx,
                async move |_: &mut AsyncWindowContext| {
                    started.set(started.get() + 1);
                    let _cancel = CancelCount(cancelled);
                    futures::future::pending::<()>().await;
                },
            );
        }
        div()
    }
}

fn task_host(cx: &mut TestAppContext) -> (WindowHandle<TaskHost>, Rc<Cell<u32>>, Rc<Cell<u32>>) {
    let started = Rc::new(Cell::new(0));
    let cancelled = Rc::new(Cell::new(0));
    let window = open(
        cx,
        TaskHost {
            shown: true,
            task_id: 1,
            started: started.clone(),
            cancelled: cancelled.clone(),
        },
    );
    (window, started, cancelled)
}

#[gpui::test]
fn task_starts_once_for_the_same_id(cx: &mut TestAppContext) {
    let (window, started, cancelled) = task_host(cx);
    assert_eq!(started.get(), 1);
    redraw(cx, window, |_| {});
    assert_eq!((started.get(), cancelled.get()), (1, 0));
}

#[gpui::test]
fn a_new_task_id_cancels_and_restarts(cx: &mut TestAppContext) {
    let (window, started, cancelled) = task_host(cx);
    redraw(cx, window, |host| host.task_id = 2);
    assert_eq!((started.get(), cancelled.get()), (2, 1));
}

#[gpui::test]
fn disappearing_cancels_the_task(cx: &mut TestAppContext) {
    let (window, started, cancelled) = task_host(cx);
    redraw(cx, window, |host| host.shown = false);
    assert_eq!((started.get(), cancelled.get()), (1, 1));
}

#[gpui::test]
fn appearing_again_restarts_the_task(cx: &mut TestAppContext) {
    let (window, started, cancelled) = task_host(cx);
    redraw(cx, window, |host| host.shown = false);
    redraw(cx, window, |host| host.shown = true);
    assert_eq!((started.get(), cancelled.get()), (2, 1));
}

type Changes = Rc<RefCell<Vec<(u32, u32)>>>;

struct ChangeHost {
    value: u32,
    initial: bool,
    changes: Changes,
}

impl Render for ChangeHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let changes = self.changes.clone();
        on_change(
            "value",
            self.value,
            self.initial,
            window,
            cx,
            move |old, new, _, _| changes.borrow_mut().push((*old, *new)),
        );
        div()
    }
}

fn change_host(cx: &mut TestAppContext, initial: bool) -> (WindowHandle<ChangeHost>, Changes) {
    let changes: Changes = Rc::default();
    let window = open(
        cx,
        ChangeHost {
            value: 1,
            initial,
            changes: changes.clone(),
        },
    );
    (window, changes)
}

#[gpui::test]
fn on_change_reports_old_and_new_after_the_frame(cx: &mut TestAppContext) {
    let (window, changes) = change_host(cx, false);
    assert!(changes.borrow().is_empty(), "without initial the first value is not a change");
    redraw(cx, window, |host| host.value = 1);
    assert!(changes.borrow().is_empty(), "an equal value is not a change");
    redraw(cx, window, |host| host.value = 5);
    redraw(cx, window, |host| host.value = 3);
    assert_eq!(*changes.borrow(), [(1, 5), (5, 3)]);
}

#[gpui::test]
fn on_change_initial_runs_once_with_old_equal_to_new(cx: &mut TestAppContext) {
    let (window, changes) = change_host(cx, true);
    redraw(cx, window, |_| {});
    assert_eq!(*changes.borrow(), [(1, 1)]);
}
