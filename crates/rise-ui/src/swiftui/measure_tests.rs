use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    Bounds, Context, IntoElement, ParentElement, Pixels, Render, Styled, TestAppContext,
    VisualTestContext, Window, div, px, size,
};

use super::measure::measure;

type Reports = Rc<RefCell<Vec<Bounds<Pixels>>>>;

struct MeasureHost {
    width: f32,
    reports: Reports,
}

impl Render for MeasureHost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let reports = self.reports.clone();
        div().child(
            div()
                .relative()
                .w(px(self.width))
                .h(px(40.))
                .child(measure("box", move |bounds, _, _| {
                    reports.borrow_mut().push(bounds)
                })),
        )
    }
}

fn reported_sizes(reports: &Reports) -> Vec<(f32, f32)> {
    reports
        .borrow()
        .iter()
        .map(|bounds| (f32::from(bounds.size.width), f32::from(bounds.size.height)))
        .collect()
}

#[gpui::test]
fn measure_reports_the_parent_bounds_only_when_they_change(cx: &mut TestAppContext) {
    let reports: Reports = Rc::default();
    let window = cx.add_window({
        let reports = reports.clone();
        move |_, _| MeasureHost {
            width: 100.,
            reports,
        }
    });
    VisualTestContext::from_window(window.into(), cx).run_until_parked();
    assert_eq!(reported_sizes(&reports), [(100., 40.)]);

    let redraw = |cx: &mut TestAppContext, width: f32| {
        let updated = window.update(cx, |host, _, cx| {
            host.width = width;
            cx.notify();
        });
        assert!(updated.is_ok(), "the test window is open");
        cx.run_until_parked();
    };
    redraw(cx, 100.);
    assert_eq!(
        reported_sizes(&reports),
        [(100., 40.)],
        "the same bounds on a later frame are not reported again"
    );
    redraw(cx, 64.);
    assert_eq!(reported_sizes(&reports), [(100., 40.), (64., 40.)]);
    assert_eq!(
        reports.borrow().last().map(|bounds| bounds.size),
        Some(size(px(64.), px(40.)))
    );
}
