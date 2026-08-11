//! A width cap belongs on a flex ROW's main axis.
//!
//! In a column, taffy measures a child's height from its resolved `w_full` and
//! clamps the width to `max_w` only afterwards, so content that reflows is
//! measured at one width and drawn at another. The box then paints past its own
//! bottom — which, inside a list, is the next row.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{Bounds, Pixels, TestAppContext, VisualTestContext, canvas, div, prelude::*, px};
use rise_theme::AppTheme;

const LIST_WIDTH: Pixels = px(1100.0);
const CONTENT_WIDTH: Pixels = px(360.0);

/// Content whose height depends on the width it is laid out at — what wrapping
/// text does, without a text system a headless test does not have.
fn reflowing_content(probe: Rc<Cell<Bounds<Pixels>>>) -> impl IntoElement {
    let mut strip = div().relative().flex().flex_wrap();
    for _ in 0..8 {
        strip = strip.child(div().w(px(100.0)).h(px(20.0)).flex_shrink_0());
    }
    strip.child(measure(probe))
}

fn measure(into: Rc<Cell<Bounds<Pixels>>>) -> impl IntoElement {
    canvas(move |bounds, _, _| into.set(bounds), |_, _: (), _, _| {})
        .absolute()
        .inset_0()
}

#[derive(Clone, Default)]
struct Measured {
    /// The box whose height a list would reserve for this row.
    reserved: Rc<Cell<Bounds<Pixels>>>,
    /// What the content actually draws once it has reflowed.
    drawn: Rc<Cell<Bounds<Pixels>>>,
}

struct Column(Measured);

impl Render for Column {
    fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div().w(LIST_WIDTH).flex().flex_col().child(
            div()
                .relative()
                .w_full()
                .flex()
                .flex_col()
                .items_center()
                .child(
                    div()
                        .w_full()
                        .max_w(CONTENT_WIDTH)
                        .child(reflowing_content(self.0.drawn.clone())),
                )
                .child(measure(self.0.reserved.clone())),
        )
    }
}

struct Row(Measured);

impl Render for Row {
    fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div().w(LIST_WIDTH).flex().flex_col().child(
            div()
                .relative()
                .w_full()
                .flex()
                .justify_center()
                .child(
                    div()
                        .w_full()
                        .max_w(CONTENT_WIDTH)
                        .flex()
                        .flex_col()
                        .child(reflowing_content(self.0.drawn.clone())),
                )
                .child(measure(self.0.reserved.clone())),
        )
    }
}

fn install(cx: &mut TestAppContext) {
    cx.update(|cx| {
        if !cx.has_global::<AppTheme>() {
            rise_ui::install_theme(AppTheme::dark(), cx);
        }
    });
}

#[gpui::test]
fn a_row_reserves_exactly_the_height_its_capped_content_draws(cx: &mut TestAppContext) {
    install(cx);

    let measured = Measured::default();
    let probe = measured.clone();
    let window = cx.add_window(|_, _| Row(probe));
    let visual = VisualTestContext::from_window(window.into(), cx);
    drop(visual);

    let drawn = measured.drawn.get();
    let reserved = measured.reserved.get();

    assert!(
        drawn.size.height > px(20.0),
        "the strip never reflowed, so this measures nothing"
    );
    assert!(
        reserved.size.height >= drawn.size.height,
        "the row reserved {:?} for content that draws {:?}, so it paints over what is below it",
        reserved.size.height,
        drawn.size.height
    );
}

#[gpui::test]
fn the_same_cap_inside_a_column_reserves_too_little(cx: &mut TestAppContext) {
    install(cx);

    let measured = Measured::default();
    let probe = measured.clone();
    let window = cx.add_window(|_, _| Column(probe));
    let visual = VisualTestContext::from_window(window.into(), cx);
    drop(visual);

    let drawn = measured.drawn.get();
    let reserved = measured.reserved.get();

    assert!(
        reserved.size.height < drawn.size.height,
        "this is the trap the row shape exists to avoid; if a taffy bump closed it \
         ({:?} reserved vs {:?} drawn), the row shape can be reconsidered",
        reserved.size.height,
        drawn.size.height
    );
}
