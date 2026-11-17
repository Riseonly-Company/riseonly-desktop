use gpui::{
    AnyElement, Context, Div, InteractiveElement, IntoElement, ParentElement, Render, Styled,
    TestAppContext, Window, div, px,
};

use crate::swiftui::stacks::{
    Alignment, AxisPlacement, DEFAULT_STACK_SPACING, HorizontalAlignment, StackAlign,
    VerticalAlignment, hstack, spacer, spacer_min_w, vstack, zstack,
};
use crate::swiftui::environment::LayoutDirection;

const ROOT: &str = "root";

struct Probe {
    build: fn() -> AnyElement,
}

impl Render for Probe {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .debug_selector(|| ROOT.to_string())
            .flex()
            .flex_row()
            .items_start()
            .w(px(400.))
            .h(px(400.))
            .child((self.build)())
    }
}

/// puts left to right back even when the harness reuses the thread for the next test
struct DirectionGuard;

impl DirectionGuard {
    fn right_to_left() -> Self {
        LayoutDirection::install(LayoutDirection::RightToLeft);
        Self
    }
}

impl Drop for DirectionGuard {
    fn drop(&mut self) {
        LayoutDirection::install(LayoutDirection::LeftToRight);
    }
}

/// x, y
/// width and height of each selector, relative to the probe root
fn measure(
    cx: &mut TestAppContext,
    build: fn() -> AnyElement,
    selectors: &[&'static str],
) -> Vec<(f32, f32, f32, f32)> {
    let (_, cx) = cx.add_window_view(|_, _| Probe { build });
    cx.run_until_parked();
    let root = cx.debug_bounds(ROOT).expect("probe root was laid out");
    selectors
        .iter()
        .copied()
        .map(|selector| {
            let bounds = cx.debug_bounds(selector).expect("probed element was laid out");
            (
                f32::from(bounds.origin.x - root.origin.x),
                f32::from(bounds.origin.y - root.origin.y),
                f32::from(bounds.size.width),
                f32::from(bounds.size.height),
            )
        })
        .collect()
}

fn leaf(selector: &'static str, width: f32, height: f32) -> Div {
    div().debug_selector(move || selector.to_string()).w(px(width)).h(px(height))
}

#[test]
fn alignment_splits_into_swiftui_axes() {
    assert_eq!(Alignment::TopLeading.horizontal(), HorizontalAlignment::Leading);
    assert_eq!(Alignment::TopLeading.vertical(), VerticalAlignment::Top);
    assert_eq!(Alignment::Bottom.horizontal(), HorizontalAlignment::Center);
    assert_eq!(Alignment::Bottom.vertical(), VerticalAlignment::Bottom);
    assert_eq!(Alignment::Trailing.horizontal(), HorizontalAlignment::Trailing);
    assert_eq!(Alignment::Trailing.vertical(), VerticalAlignment::Center);
    assert_eq!(Alignment::default(), Alignment::Center);
}

#[test]
fn leading_and_trailing_mirror_right_to_left() {
    let ltr = LayoutDirection::LeftToRight;
    let rtl = LayoutDirection::RightToLeft;
    assert_eq!(HorizontalAlignment::Leading.placement(ltr), AxisPlacement::Start);
    assert_eq!(HorizontalAlignment::Leading.placement(rtl), AxisPlacement::End);
    assert_eq!(HorizontalAlignment::Trailing.placement(ltr), AxisPlacement::End);
    assert_eq!(HorizontalAlignment::Trailing.placement(rtl), AxisPlacement::Start);
    assert_eq!(HorizontalAlignment::Center.placement(rtl), AxisPlacement::Center);
    assert_eq!(LayoutDirection::from_is_rtl(true), rtl);
    assert_eq!(LayoutDirection::from_is_rtl(false), ltr);
}

#[gpui::test]
fn zstack_takes_the_size_of_its_largest_child(cx: &mut TestAppContext) {
    let measured = measure(
        cx,
        || {
            zstack(Alignment::Center)
                .debug_selector(|| "z".to_string())
                .child(leaf("wide", 40., 10.))
                .child(leaf("tall", 20., 30.))
                .into_any_element()
        },
        &["z", "wide", "tall"],
    );
    assert_eq!(measured[0], (0., 0., 40., 30.));
    assert_eq!(measured[1], (0., 10., 40., 10.));
    assert_eq!(measured[2], (10., 0., 20., 30.));
}

#[gpui::test]
fn zstack_places_children_at_its_alignment(cx: &mut TestAppContext) {
    let measured = measure(
        cx,
        || {
            zstack(Alignment::BottomTrailing)
                .child(leaf("big", 50., 50.))
                .child(leaf("badge", 10., 10.))
                .into_any_element()
        },
        &["big", "badge"],
    );
    assert_eq!(measured[0], (0., 0., 50., 50.));
    assert_eq!(measured[1], (40., 40., 10., 10.));
}

#[gpui::test]
fn zstack_leading_sits_on_the_right_right_to_left(cx: &mut TestAppContext) {
    let _rtl = DirectionGuard::right_to_left();
    let measured = measure(
        cx,
        || {
            zstack(Alignment::TopLeading)
                .child(leaf("big", 50., 50.))
                .child(leaf("badge", 10., 10.))
                .into_any_element()
        },
        &["badge"],
    );
    assert_eq!(measured[0], (40., 0., 10., 10.));
}

#[gpui::test]
fn vstack_centres_children_with_the_default_spacing(cx: &mut TestAppContext) {
    let measured = measure(
        cx,
        || vstack(None).child(leaf("a", 40., 10.)).child(leaf("b", 20., 10.)).into_any_element(),
        &["a", "b"],
    );
    assert_eq!(measured[0], (0., 0., 40., 10.));
    assert_eq!(measured[1], (10., 10. + DEFAULT_STACK_SPACING, 20., 10.));
}

#[gpui::test]
fn vstack_leading_alignment_follows_the_layout_direction(cx: &mut TestAppContext) {
    fn build() -> AnyElement {
        vstack(4.)
            .align(HorizontalAlignment::Leading)
            .child(leaf("a", 40., 10.))
            .child(leaf("b", 20., 10.))
            .into_any_element()
    }
    assert_eq!(measure(cx, build, &["b"])[0], (0., 14., 20., 10.));

    let _rtl = DirectionGuard::right_to_left();
    assert_eq!(measure(cx, build, &["b"])[0], (20., 14., 20., 10.));
}

#[gpui::test]
fn hstack_runs_from_the_right_edge_right_to_left(cx: &mut TestAppContext) {
    fn build() -> AnyElement {
        hstack(0.)
            .align(VerticalAlignment::Top)
            .child(leaf("first", 30., 10.))
            .child(leaf("second", 20., 20.))
            .into_any_element()
    }
    let ltr = measure(cx, build, &["first", "second"]);
    assert_eq!(ltr[0], (0., 0., 30., 10.));
    assert_eq!(ltr[1], (30., 0., 20., 20.));

    let _rtl = DirectionGuard::right_to_left();
    let rtl = measure(cx, build, &["first", "second"]);
    assert_eq!(rtl[0], (20., 0., 30., 10.));
    assert_eq!(rtl[1], (0., 0., 20., 20.));
}

#[gpui::test]
fn spacer_takes_the_free_space_of_an_hstack(cx: &mut TestAppContext) {
    let measured = measure(
        cx,
        || {
            hstack(0.)
                .w(px(100.))
                .child(leaf("title", 30., 10.))
                .child(spacer())
                .child(leaf("trailing", 20., 10.))
                .into_any_element()
        },
        &["trailing"],
    );
    assert_eq!(measured[0], (80., 0., 20., 10.));
}

#[gpui::test]
fn spacer_min_length_adds_to_the_stack_spacing(cx: &mut TestAppContext) {
    let measured = measure(
        cx,
        || {
            hstack(4.)
                .child(leaf("a", 30., 10.))
                .child(spacer_min_w(10.))
                .child(leaf("b", 20., 10.))
                .into_any_element()
        },
        &["b"],
    );
    assert_eq!(measured[0], (48., 0., 20., 10.));
}

#[gpui::test]
fn a_scoped_direction_holds_for_a_zstack_laid_out_later(cx: &mut TestAppContext) {
    let measured = measure(
        cx,
        || {
            LayoutDirection::scoped(LayoutDirection::RightToLeft, || {
                zstack(Alignment::TopLeading)
                    .child(leaf("big", 50., 50.))
                    .child(leaf("badge", 10., 10.))
                    .into_any_element()
            })
        },
        &["badge"],
    );
    assert_eq!(measured[0], (40., 0., 10., 10.));
    assert_eq!(LayoutDirection::current(), LayoutDirection::LeftToRight);
}
