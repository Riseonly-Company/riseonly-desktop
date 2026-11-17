use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui::{
    App, Context, IntoElement, ParentElement, Render, RenderOnce, TestAppContext,
    VisualTestContext, Window, div,
};

use super::dismiss::{DismissAction, DismissKey, dismiss};
use super::environment::env_scope;

type Captured = Rc<RefCell<Vec<DismissAction>>>;

#[derive(IntoElement)]
struct SheetContent {
    captured: Captured,
}

impl RenderOnce for SheetContent {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        self.captured.borrow_mut().push(dismiss(cx));
        div()
    }
}

struct SheetHost {
    closed: Rc<Cell<u32>>,
    captured: Captured,
}

impl Render for SheetHost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let closed = self.closed.clone();
        env_scope::<DismissKey>(DismissAction::new(move |_, _| closed.set(closed.get() + 1)))
            .child(SheetContent {
                captured: self.captured.clone(),
            })
    }
}

#[gpui::test]
fn content_dismisses_the_host_that_presented_it(cx: &mut TestAppContext) {
    let closed = Rc::new(Cell::new(0));
    let captured: Captured = Rc::new(RefCell::new(Vec::new()));
    let host_closed = closed.clone();
    let host_captured = captured.clone();
    let window = cx.add_window(|_, _| SheetHost {
        closed: host_closed,
        captured: host_captured,
    });
    let visual = VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();

    let action = captured.borrow().first().cloned();
    let Some(action) = action else {
        panic!("the sheet content never rendered");
    };
    window
        .update(cx, |_, window, cx| action.call_as_function(window, cx))
        .unwrap();
    assert_eq!(closed.get(), 1);
}

#[gpui::test]
fn dismiss_outside_a_presentation_does_nothing(cx: &mut TestAppContext) {
    let closed = Rc::new(Cell::new(0));
    let host_closed = closed.clone();
    let window = cx.add_window(|_, _| SheetHost {
        closed: host_closed,
        captured: Rc::new(RefCell::new(Vec::new())),
    });

    let action = cx.update(|cx| dismiss(cx));
    window
        .update(cx, |_, window, cx| action.call_as_function(window, cx))
        .unwrap();
    assert_eq!(closed.get(), 0);
}
