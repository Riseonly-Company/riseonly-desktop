//! A list only exists once it has been measured: gpui keeps its rows out of the
//! layout tree, so nothing but a real window can say whether they were painted.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    ListAlignment, ScrollDelta, ScrollWheelEvent, TestAppContext, VisualTestContext, div, point,
    prelude::*, px,
};
use rise_theme::AppTheme;
use rise_ui::{EdgeState, ListUi, PaginationEdge};

struct Feed {
    list: ListUi,
    painted: Rc<Cell<usize>>,
}

impl Render for Feed {
    fn render(
        &mut self,
        _window: &mut gpui::Window,
        _cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        let painted = self.painted.clone();

        // The screen shell's geometry, as ScreenShellUi::render builds it: a
        // fixed header, then the content filling what is left of the column.
        div()
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(div().flex_shrink_0().h(px(56.0)))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(div().size_full().child(self.list.element(move |row, _, _| {
                        painted.set(painted.get() + 1);
                        div()
                            .h(px(120.0))
                            .child(format!("row {row}"))
                            .into_any_element()
                    }))),
            )
    }
}

#[gpui::test]
fn a_list_of_rows_is_measured_and_painted_rather_than_collapsing_to_nothing(
    cx: &mut TestAppContext,
) {
    cx.update(|cx| {
        if !cx.has_global::<AppTheme>() {
            rise_ui::install_theme(AppTheme::dark(), cx);
        }
    });

    let painted = Rc::new(Cell::new(0));
    let probe = painted.clone();

    let window = cx.add_window(|_, cx| {
        let theme = rise_ui::theme(cx as &gpui::App).clone();
        Feed {
            list: ListUi::new(&theme, 40, ListAlignment::Top),
            painted: probe,
        }
    });
    let visual = VisualTestContext::from_window(window.into(), cx);
    drop(visual);

    let height = window
        .update(cx, |view, _, _| {
            view.list.state().viewport_bounds().size.height
        })
        .unwrap();

    assert!(
        height > px(0.0),
        "an unsized list measures to nothing, so a screen holding forty rows draws an empty page"
    );
    assert!(
        painted.get() > 0,
        "the viewport has height but no row was asked for"
    );
}

#[gpui::test]
fn scrolling_an_armed_list_does_not_take_the_process_down(cx: &mut TestAppContext) {
    cx.update(|cx| {
        if !cx.has_global::<AppTheme>() {
            rise_ui::install_theme(AppTheme::dark(), cx);
        }
    });

    let reached = Rc::new(Cell::new(0));
    let counted = reached.clone();

    let window = cx.add_window(|_, cx| {
        let theme = rise_ui::theme(cx as &gpui::App).clone();
        Feed {
            list: ListUi::new(&theme, 200, ListAlignment::Top),
            painted: Rc::new(Cell::new(0)),
        }
    });

    window
        .update(cx, |view, _, cx| {
            let entity = cx.entity();
            view.list.on_edge(
                &entity,
                |_: &Feed, _| EdgeState {
                    has_more_top: false,
                    has_more_bottom: true,
                    top_in_flight: false,
                    bottom_in_flight: false,
                },
                move |_: &mut Feed, edge, _| {
                    if edge == PaginationEdge::Bottom {
                        counted.set(counted.get() + 1);
                    }
                },
            );
        })
        .unwrap();

    let mut visual = VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();

    // gpui keeps the list's RefCell borrowed for the whole scroll callback, so a
    // policy that asks the list where it is aborts the process rather than panicking.
    for _ in 0..8 {
        visual.simulate_event(ScrollWheelEvent {
            position: point(px(200.0), px(300.0)),
            delta: ScrollDelta::Pixels(point(px(0.0), px(-400.0))),
            ..Default::default()
        });
        visual.run_until_parked();
    }

    let scrolled = window
        .update(cx, |view, _, _| {
            -view.list.state().scroll_px_offset_for_scrollbar().y
        })
        .unwrap();
    assert!(
        scrolled > px(0.0),
        "the wheel never reached the list, so this proves nothing about the scroll path"
    );
    assert!(
        reached.get() > 0,
        "the bottom edge never armed, so the policy was never exercised"
    );
}
