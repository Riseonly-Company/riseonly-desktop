//! A click on an overlay's own content must reach it, not the scrim behind it.
//!
//! Every other test in this crate drives the state machine directly, which
//! cannot see this: the defect is entirely in the element tree, where gpui's hit
//! test reports EVERY hitbox under the pointer and the window-filling scrim is
//! one of them. So this one goes through a real window and a real click.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{Modifiers, TestAppContext, VisualTestContext, div, point, prelude::*, px, size};
use rise_theme::AppTheme;
use rise_widgets::{OverlayAnchor, OverlayLayer, OverlayScrim, OverlaySide, place};

struct Harness {
    clicked: Rc<Cell<usize>>,
    dismissed: Rc<Cell<usize>>,
}

impl Render for Harness {
    fn render(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        let clicked = self.clicked.clone();
        let dismissed = self.dismissed.clone();

        let placement = place(
            OverlayAnchor::Point(point(px(100.0), px(100.0))),
            size(px(200.0), px(120.0)),
            window.viewport_size(),
            px(4.0),
            px(12.0),
            OverlaySide::Below,
        );

        let content = div()
            .id("overlay.row")
            .w(px(200.0))
            .h(px(120.0))
            .on_click(move |_, _, _| clicked.set(clicked.get() + 1));

        div().size_full().relative().child(
            OverlayLayer::new(placement)
                .scrim(OverlayScrim::Dimmed)
                .child(content)
                .on_dismiss(move |_, _| dismissed.set(dismissed.get() + 1))
                .render(cx),
        )
    }
}

fn open(cx: &mut TestAppContext) -> (Rc<Cell<usize>>, Rc<Cell<usize>>, VisualTestContext) {
    cx.update(|cx| {
        if !cx.has_global::<AppTheme>() {
            rise_ui::install_theme(AppTheme::dark(), cx);
        }
    });

    let clicked = Rc::new(Cell::new(0));
    let dismissed = Rc::new(Cell::new(0));
    let window = cx.add_window(|_, _| Harness {
        clicked: clicked.clone(),
        dismissed: dismissed.clone(),
    });

    let visual = VisualTestContext::from_window(window.into(), cx);
    (clicked, dismissed, visual)
}

#[gpui::test]
fn a_click_on_the_overlay_reaches_it_instead_of_dismissing_it(cx: &mut TestAppContext) {
    let (clicked, dismissed, mut visual) = open(cx);

    visual.simulate_click(point(px(150.0), px(150.0)), Modifiers::default());

    assert_eq!(
        clicked.get(),
        1,
        "the overlay's own content never saw the click"
    );
    assert_eq!(
        dismissed.get(),
        0,
        "the scrim dismissed while the pointer was over the overlay, which would \
         tear the menu down before the click could complete"
    );
}

#[gpui::test]
fn a_click_outside_the_overlay_still_dismisses_it(cx: &mut TestAppContext) {
    let (clicked, dismissed, mut visual) = open(cx);

    visual.simulate_click(point(px(600.0), px(500.0)), Modifiers::default());

    assert_eq!(
        dismissed.get(),
        1,
        "clicking away must still close the overlay"
    );
    assert_eq!(clicked.get(), 0);
}
