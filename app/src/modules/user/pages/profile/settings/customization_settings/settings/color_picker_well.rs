use std::sync::Arc;

use gpui::{App, Context, ElementId, Hsla, SharedString, Window, anchored, hsla, point};
use rise_ui::swiftui::angular_gradient::{AngularGradient, angular_gradient_circle_stroke};
use rise_ui::swiftui::prelude::*;

use crate::modules::media_picker::editor::editor_color_picker_bridge::{
    EditorColorPickerBridge, EditorColorPickerHost, EditorColorPickerLauncher,
};

const WELL_SIDE: f32 = 28.0;
const RING_WIDTH: f32 = 3.0;
const SWATCH_SIDE: f32 = 18.0;
const RING_STOPS: u8 = 6;

fn named_child(id: &ElementId, name: &'static str) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), SharedString::new_static(name))
}

/// system style color well without opacity and labels
#[derive(IntoElement)]
pub struct ColorPickerWell {
    id: ElementId,
    selection: Binding<Hsla>,
    side: f32,
}

impl ColorPickerWell {
    pub fn new(id: impl Into<ElementId>, selection: Binding<Hsla>) -> Self {
        Self { id: id.into(), selection, side: WELL_SIDE }
    }

    /// side of the picker box, the ring is drawn on it
    pub fn side(mut self, side: f32) -> Self {
        self.side = side;
        self
    }
}

impl RenderOnce for ColorPickerWell {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let presented = window.use_keyed_state(named_child(&self.id, "presented"), cx, |_, _| false);
        let host = window.use_keyed_state(
            named_child(&self.id, "host"),
            cx,
            |_, cx: &mut Context<EditorColorPickerHost>| EditorColorPickerHost::new(cx),
        );
        let is_presented =
            *presented.read(cx) && EditorColorPickerBridge::shared(cx).read(cx).picker_in_flight().is_some();

        let current = self.selection.get(cx);
        let live_selection = self.selection.clone();
        let ring = AngularGradient::new((0..=RING_STOPS).map(|stop| hsla(f32::from(stop) / f32::from(RING_STOPS), 1.0, 0.5, 1.0)));

        let mut root = div().child(
            plain(named_child(&self.id, "well"))
                .w(px(self.side))
                .h(px(self.side))
                .on_press(move |window, cx| {
                    let initial = live_selection.get(cx);
                    let selection = live_selection.clone();
                    let finished = presented.clone();
                    presented.update(cx, |presented, cx| {
                        *presented = true;
                        cx.notify();
                    });
                    EditorColorPickerLauncher::present(
                        initial,
                        false,
                        move |color, window, cx| selection.set(color, window, cx),
                        move |_, _, cx| {
                            finished.update(cx, |presented, cx| {
                                *presented = false;
                                cx.notify();
                            });
                        },
                        window,
                        cx,
                    );
                })
                .child(
                    zstack(Alignment::Center)
                        .child(angular_gradient_circle_stroke(ring, RING_WIDTH, self.side))
                        .child(div().w(px(SWATCH_SIDE)).h(px(SWATCH_SIDE)).rounded_full().bg(current)),
                ),
        );
        if is_presented {
            let viewport = window.viewport_size();
            root = root.child(
                anchored()
                    .position(point(px(0.0), px(0.0)))
                    .child(div().relative().w(viewport.width).h(viewport.height).child(host)),
            );
        }
        root
    }
}
