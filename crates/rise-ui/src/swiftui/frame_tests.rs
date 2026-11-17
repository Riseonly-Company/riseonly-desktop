use gpui::{
    AnyElement, Context, Div, InteractiveElement, IntoElement, ParentElement, Render, Styled,
    TestAppContext, Window, div, px,
};

use crate::swiftui::frame::{DEFAULT_PADDING, EdgeSet, SwiftUiAlignedFrame, SwiftUiFrame};
use crate::swiftui::environment::LayoutDirection;
use crate::swiftui::stacks::{Alignment, vstack};

const ROOT: &str = "root";

struct Probe {
    build: fn() -> AnyElement,
}

impl Render for Probe {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .debug_selector(|| ROOT.to_string())
            .flex()
            .flex_col()
            .items_start()
            .w(px(200.))
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

#[gpui::test]
fn frame_max_width_fills_the_parent_and_aligns_its_content(cx: &mut TestAppContext) {
    let leading = measure(
        cx,
        || {
            leaf("content", 40., 10.)
                .frame_max_width(Alignment::Leading)
                .debug_selector(|| "frame".to_string())
                .into_any_element()
        },
        &["frame", "content"],
    );
    assert_eq!(leading[0], (0., 0., 200., 10.));
    assert_eq!(leading[1], (0., 0., 40., 10.));

    let trailing = measure(
        cx,
        || leaf("content", 40., 10.).frame_max_width(Alignment::Trailing).into_any_element(),
        &["content"],
    );
    assert_eq!(trailing[0], (160., 0., 40., 10.));

    let centred = measure(
        cx,
        || leaf("content", 40., 10.).frame_max_width(Alignment::Center).into_any_element(),
        &["content"],
    );
    assert_eq!(centred[0], (80., 0., 40., 10.));
}

#[gpui::test]
fn frame_max_width_leading_mirrors_right_to_left(cx: &mut TestAppContext) {
    let _rtl = DirectionGuard::right_to_left();
    let measured = measure(
        cx,
        || leaf("content", 40., 10.).frame_max_width(Alignment::Leading).into_any_element(),
        &["content"],
    );
    assert_eq!(measured[0], (160., 0., 40., 10.));
}

#[gpui::test]
fn frame_keeps_the_stack_alignment_of_its_content(cx: &mut TestAppContext) {
    let measured = measure(
        cx,
        || {
            vstack(0.)
                .debug_selector(|| "stack".to_string())
                .child(leaf("wide", 40., 10.))
                .child(leaf("narrow", 20., 10.))
                .frame_max_width(Alignment::Leading)
                .into_any_element()
        },
        &["stack", "narrow"],
    );
    assert_eq!(measured[0], (0., 0., 40., 20.));
    assert_eq!(measured[1], (10., 10., 20., 10.));
}

#[gpui::test]
fn frame_size_aligned_places_smaller_content(cx: &mut TestAppContext) {
    let measured = measure(
        cx,
        || {
            leaf("icon", 20., 20.)
                .frame_size_aligned(44., 44., Alignment::Center)
                .debug_selector(|| "frame".to_string())
                .into_any_element()
        },
        &["frame", "icon"],
    );
    assert_eq!(measured[0], (0., 0., 44., 44.));
    assert_eq!(measured[1], (12., 12., 20., 20.));
}

#[gpui::test]
fn frame_size_leaves_an_unset_axis_to_the_content(cx: &mut TestAppContext) {
    let measured = measure(
        cx,
        || {
            div()
                .debug_selector(|| "box".to_string())
                .frame_size(60., None)
                .child(leaf("content", 10., 25.))
                .into_any_element()
        },
        &["box"],
    );
    assert_eq!(measured[0], (0., 0., 60., 25.));
}

#[gpui::test]
fn padding_edges_follow_the_layout_direction(cx: &mut TestAppContext) {
    fn build() -> AnyElement {
        div()
            .flex()
            .padding_edges(EdgeSet::LEADING | EdgeSet::TOP, 8.)
            .child(leaf("content", 10., 10.))
            .into_any_element()
    }
    assert_eq!(measure(cx, build, &["content"])[0], (8., 8., 10., 10.));

    let _rtl = DirectionGuard::right_to_left();
    let measured = measure(
        cx,
        || {
            div()
                .flex()
                .w(px(50.))
                .padding_edges(EdgeSet::LEADING, 8.)
                .child(leaf("content", 10., 10.))
                .into_any_element()
        },
        &["content"],
    );
    assert_eq!(measured[0], (0., 0., 10., 10.));
}

#[gpui::test]
fn padding_default_is_sixteen_on_every_edge(cx: &mut TestAppContext) {
    assert_eq!(DEFAULT_PADDING, 16.);
    let measured = measure(
        cx,
        || {
            div()
                .debug_selector(|| "box".to_string())
                .padding_default()
                .child(leaf("content", 10., 10.))
                .into_any_element()
        },
        &["box", "content"],
    );
    assert_eq!(measured[0], (0., 0., 42., 42.));
    assert_eq!(measured[1], (16., 16., 10., 10.));
}

#[test]
fn edge_sets_combine_like_swiftui() {
    assert_eq!(EdgeSet::LEADING | EdgeSet::TRAILING, EdgeSet::HORIZONTAL);
    assert_eq!(EdgeSet::TOP | EdgeSet::BOTTOM, EdgeSet::VERTICAL);
    assert_eq!(EdgeSet::HORIZONTAL | EdgeSet::VERTICAL, EdgeSet::ALL);
}
