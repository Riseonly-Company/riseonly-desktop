use std::cell::Cell;
use std::rc::Rc;

use gpui::{App, Context, IntoElement, ParentElement as _, Render, RenderOnce, TestAppContext, VisualTestContext, Window, div};
use rise_i18n::language_store::LanguageStore;
use rise_ui::swiftui::environment::{LayoutDirection, layout_direction};

use super::RiseonlyApp;

const RTL_LANGUAGE: &str = "ar";
const LTR_LANGUAGE: &str = "en";

fn select(code: &str, cx: &mut TestAppContext) {
    cx.update(|cx| LanguageStore::update(cx, |store, cx| store.select_language(code, cx)));
    cx.run_until_parked();
}

#[derive(IntoElement)]
struct DirectionProbe {
    seen: Rc<Cell<Option<LayoutDirection>>>,
}

impl RenderOnce for DirectionProbe {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        self.seen.set(Some(layout_direction(cx)));
        div()
    }
}

struct BodyProbe {
    seen: Rc<Cell<Option<LayoutDirection>>>,
}

impl Render for BodyProbe {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = div().child(DirectionProbe { seen: self.seen.clone() }).into_any_element();
        RiseonlyApp::body(content, cx)
    }
}

#[gpui::test]
fn the_installed_direction_follows_the_language(cx: &mut TestAppContext) {
    cx.update(RiseonlyApp::install_layout_direction);
    select(RTL_LANGUAGE, cx);
    assert_eq!(LayoutDirection::current(), LayoutDirection::RightToLeft);
    select(LTR_LANGUAGE, cx);
    assert_eq!(LayoutDirection::current(), LayoutDirection::LeftToRight);
}

#[gpui::test]
fn the_body_sets_the_environment_from_the_language(cx: &mut TestAppContext) {
    select(RTL_LANGUAGE, cx);
    let seen = Rc::new(Cell::new(None));
    let probe = seen.clone();
    let window = cx.add_window(|_, _| BodyProbe { seen: probe });
    let visual = VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    assert_eq!(LayoutDirection::current(), LayoutDirection::LeftToRight);
    assert_eq!(seen.get(), Some(LayoutDirection::RightToLeft));
    select(LTR_LANGUAGE, cx);
}
