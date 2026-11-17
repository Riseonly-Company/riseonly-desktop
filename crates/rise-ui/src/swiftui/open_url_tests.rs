use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui::{
    App, Context, IntoElement, ParentElement, Render, RenderOnce, TestAppContext,
    VisualTestContext, Window, WindowHandle, div,
};
use rise_core::foundation::foundation_url::Url;
use rise_platform::open_url::install_in_app_url_route;

use super::environment::env_scope;
use super::open_url::{OpenURLAction, OpenURLActionResult, OpenURLKey, open_url};

const SCHEME: &str = "riseonly";

type Captured = Rc<RefCell<Vec<OpenURLAction>>>;
type Routed = Rc<RefCell<Vec<String>>>;

fn url(raw: &str) -> Url {
    let Some(url) = Url::from_string(raw) else {
        panic!("{raw} is not a URL");
    };
    url
}

struct Blank;

impl Render for Blank {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

fn install_router(cx: &mut TestAppContext) -> Routed {
    let routed: Routed = Rc::new(RefCell::new(Vec::new()));
    let sink = routed.clone();
    cx.update(|cx| {
        install_in_app_url_route(
            SCHEME,
            move |url, _| sink.borrow_mut().push(url.absolute_string().to_owned()),
            cx,
        )
    });
    routed
}

fn open(
    action: &OpenURLAction,
    raw: &str,
    window: WindowHandle<impl Render>,
    cx: &mut TestAppContext,
) -> bool {
    let accepted = Rc::new(Cell::new(None));
    let report = accepted.clone();
    let target = url(raw);
    window
        .update(cx, |_, window, cx| {
            action.call_as_function_completion(&target, window, cx, |ok| report.set(Some(ok)))
        })
        .unwrap();
    let Some(accepted) = accepted.get() else {
        panic!("the completion never ran for {raw}");
    };
    accepted
}

#[gpui::test]
fn own_deep_link_goes_to_the_router_and_not_to_the_system(cx: &mut TestAppContext) {
    let routed = install_router(cx);
    let window = cx.add_window(|_, _| Blank);

    let action = cx.update(|cx| open_url(cx));
    assert!(open(&action, "riseonly://chat/42", window, cx));
    assert!(open(&action, "https://riseonly.net/post/9", window, cx));

    assert_eq!(
        *routed.borrow(),
        vec![
            "riseonly://chat/42".to_owned(),
            "https://riseonly.net/post/9".to_owned()
        ]
    );
    assert_eq!(cx.opened_url(), None);
}

#[gpui::test]
fn a_foreign_web_link_goes_to_the_system(cx: &mut TestAppContext) {
    let routed = install_router(cx);
    let window = cx.add_window(|_, _| Blank);

    let action = cx.update(|cx| open_url(cx));
    assert!(open(&action, "https://example.com/a", window, cx));

    assert!(routed.borrow().is_empty());
    assert_eq!(cx.opened_url(), Some("https://example.com/a".to_owned()));
}

#[gpui::test]
fn a_file_link_is_refused(cx: &mut TestAppContext) {
    install_router(cx);
    let window = cx.add_window(|_, _| Blank);

    let action = cx.update(|cx| open_url(cx));
    assert!(!open(&action, "file:///etc/passwd", window, cx));
    assert_eq!(cx.opened_url(), None);
}

#[gpui::test]
fn handler_results_decide_what_opens(cx: &mut TestAppContext) {
    let routed = install_router(cx);
    let window = cx.add_window(|_, _| Blank);

    let handled = OpenURLAction::new(|_, _, _| OpenURLActionResult::Handled);
    assert!(open(&handled, "https://example.com/a", window, cx));
    let discarded = OpenURLAction::new(|_, _, _| OpenURLActionResult::Discarded);
    assert!(!open(&discarded, "https://example.com/a", window, cx));
    assert_eq!(cx.opened_url(), None);

    let replaced = OpenURLAction::new(|_, _, _| {
        OpenURLActionResult::SystemActionUrl(url("riseonly://post/9"))
    });
    assert!(open(&replaced, "https://example.com/a", window, cx));
    assert_eq!(*routed.borrow(), vec!["riseonly://post/9".to_owned()]);
    assert_eq!(cx.opened_url(), None);
}

#[derive(IntoElement)]
struct LinkText {
    captured: Captured,
}

impl RenderOnce for LinkText {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        self.captured.borrow_mut().push(open_url(cx));
        div()
    }
}

struct HashtagHost {
    tapped: Rc<Cell<u32>>,
    captured: Captured,
}

impl Render for HashtagHost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let tapped = self.tapped.clone();
        env_scope::<OpenURLKey>(OpenURLAction::new(move |_, _, _| {
            tapped.set(tapped.get() + 1);
            OpenURLActionResult::Handled
        }))
        .child(LinkText {
            captured: self.captured.clone(),
        })
    }
}

#[gpui::test]
fn a_subtree_handler_is_what_its_views_open_with(cx: &mut TestAppContext) {
    install_router(cx);
    let tapped = Rc::new(Cell::new(0));
    let captured: Captured = Rc::new(RefCell::new(Vec::new()));
    let host_tapped = tapped.clone();
    let host_captured = captured.clone();
    let window = cx.add_window(|_, _| HashtagHost {
        tapped: host_tapped,
        captured: host_captured,
    });
    let visual = VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();

    let action = captured.borrow().first().cloned();
    let Some(action) = action else {
        panic!("the link text never rendered");
    };
    assert!(open(&action, "https://example.com/a", window, cx));
    assert_eq!(tapped.get(), 1);
    assert_eq!(cx.opened_url(), None);
}
