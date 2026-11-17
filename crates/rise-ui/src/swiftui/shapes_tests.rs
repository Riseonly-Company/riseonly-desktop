use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AnyElement, Context, Div, InteractiveElement, IntoElement, Modifiers, ParentElement, Render,
    StatefulInteractiveElement, Styled, TestAppContext, VisualTestContext, Window, black, div, point,
    px,
};

use crate::swiftui::shapes::{
    RoundedCornerStyle, Shape, SwiftUiShapes, centred_stroke, hit_area, pass_through,
};

const ROOT: &str = "root";

struct Probe {
    build: Box<dyn Fn() -> AnyElement>,
}

impl Render for Probe {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .debug_selector(|| ROOT.to_string())
            .flex()
            .flex_col()
            .items_start()
            .p(px(20.))
            .w(px(300.))
            .h(px(300.))
            .child((self.build)())
    }
}

fn open(
    cx: &mut TestAppContext,
    build: impl Fn() -> AnyElement + 'static,
) -> &mut VisualTestContext {
    let (_, cx) = cx.add_window_view(move |_, _| Probe { build: Box::new(build) });
    cx.run_until_parked();
    cx
}

/// x, y
/// width and height of the selector, relative to the probe root
fn bounds_of(cx: &mut VisualTestContext, selector: &'static str) -> (f32, f32, f32, f32) {
    let root = cx.debug_bounds(ROOT).expect("probe root was laid out");
    let bounds = cx.debug_bounds(selector).expect("probed element was laid out");
    (
        f32::from(bounds.origin.x - root.origin.x),
        f32::from(bounds.origin.y - root.origin.y),
        f32::from(bounds.size.width),
        f32::from(bounds.size.height),
    )
}

fn click_at(cx: &mut VisualTestContext, x: f32, y: f32) {
    let root = cx.debug_bounds(ROOT).expect("probe root was laid out");
    cx.simulate_click(point(root.origin.x + px(x), root.origin.y + px(y)), Modifiers::none());
    cx.run_until_parked();
}

fn leaf(selector: &'static str, width: f32, height: f32) -> Div {
    div().debug_selector(move || selector.to_string()).w(px(width)).h(px(height))
}

#[test]
fn outset_grows_the_corner_radius_by_half_the_line() {
    assert_eq!(Shape::continuous(12.).outset(0.5), Shape::continuous(12.5));
    assert_eq!(Shape::rounded_rect(8.).outset(1.), Shape::rounded_rect(9.));
    assert_eq!(Shape::Rectangle.outset(2.), Shape::Rectangle);
    assert_eq!(Shape::Capsule.outset(2.), Shape::Capsule);
}

#[test]
fn continuous_keeps_its_corner_style() {
    assert_eq!(
        Shape::continuous(16.),
        Shape::RoundedRectangle { corner_radius: 16., style: RoundedCornerStyle::Continuous }
    );
    assert_eq!(
        Shape::rounded_rect(16.),
        Shape::RoundedRectangle { corner_radius: 16., style: RoundedCornerStyle::Circular }
    );
}

#[gpui::test]
fn circle_is_square(cx: &mut TestAppContext) {
    let cx = open(cx, || {
        div().debug_selector(|| "circle".to_string()).w(px(40.)).circle().into_any_element()
    });
    assert_eq!(bounds_of(cx, "circle"), (20., 20., 40., 40.));
}

#[gpui::test]
fn centred_stroke_lies_half_outside_the_bounds(cx: &mut TestAppContext) {
    let cx = open(cx, || {
        leaf("shape", 100., 40.)
            .relative()
            .rounded_rect(12.)
            .child(
                centred_stroke(black(), 2., Shape::rounded_rect(12.))
                    .debug_selector(|| "stroke".to_string()),
            )
            .into_any_element()
    });
    assert_eq!(bounds_of(cx, "shape"), (20., 20., 100., 40.));
    assert_eq!(bounds_of(cx, "stroke"), (19., 19., 102., 42.));
}

#[gpui::test]
fn stroke_border_keeps_the_bounds(cx: &mut TestAppContext) {
    let cx = open(cx, || {
        leaf("shape", 100., 40.).capsule().stroke_border(black(), 3.).into_any_element()
    });
    assert_eq!(bounds_of(cx, "shape"), (20., 20., 100., 40.));
}

#[gpui::test]
fn hit_area_takes_taps_in_the_empty_space_of_a_row(cx: &mut TestAppContext) {
    let taps = Rc::new(Cell::new(0));
    let counter = taps.clone();
    let cx = open(cx, move || {
        let counter = counter.clone();
        div()
            .debug_selector(|| "row".to_string())
            .relative()
            .flex()
            .flex_row()
            .w(px(200.))
            .h(px(44.))
            .child(leaf("label", 40., 20.))
            .child(
                hit_area(Shape::Rectangle)
                    .debug_selector(|| "hit".to_string())
                    .id("row-hit")
                    .on_click(move |_, _, _| counter.set(counter.get() + 1)),
            )
            .into_any_element()
    });
    assert_eq!(bounds_of(cx, "hit"), (20., 20., 200., 44.));
    click_at(cx, 180., 40.);
    assert_eq!(taps.get(), 1);
}

#[gpui::test]
fn pass_through_lets_the_click_reach_the_element_under_it(cx: &mut TestAppContext) {
    let taps = Rc::new(Cell::new(0));
    let counter = taps.clone();
    let cx = open(cx, move || {
        let counter = counter.clone();
        div()
            .relative()
            .w(px(100.))
            .h(px(40.))
            .child(
                div()
                    .id("button")
                    .size_full()
                    .on_click(move |_, _, _| counter.set(counter.get() + 1)),
            )
            .child(
                pass_through()
                    .absolute()
                    .inset_0()
                    .bg(black())
                    .child(leaf("overlay", 100., 40.)),
            )
            .into_any_element()
    });
    assert_eq!(bounds_of(cx, "overlay"), (20., 20., 100., 40.));
    click_at(cx, 70., 40.);
    assert_eq!(taps.get(), 1);
}
