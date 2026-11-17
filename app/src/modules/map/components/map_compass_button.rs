use std::f32::consts::PI;

use gpui::{
    App, Bounds, Context, ElementId, Hsla, IntoElement, ParentElement, PathBuilder, Pixels, Render,
    RenderOnce, StyleRefinement, Styled, Window, canvas, div, point, px,
};
use rise_i18n::tr;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::page_header_toolbar_button::PageHeaderMetrics;

use crate::modules::map::components::globe::map_globe_camera_link::MapGlobeCameraLink;
use crate::modules::map::stores::map_interactions::map_interactions::MapInteractionsStore;

const SYSTEM_RED: Hsla = Hsla {
    h: 3.0 / 360.0,
    s: 1.0,
    l: 0.594,
    a: 1.0,
};

#[derive(IntoElement)]
pub struct MapCompassButton;

impl MapCompassButton {
    pub fn new() -> Self {
        Self
    }
}

impl Default for MapCompassButton {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for MapCompassButton {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let color = rise_ui::theme(cx).text.primary;
        LiquidGlassButton::new("map-compass-button", LiquidGlassButtonShape::Circle)
            .size(PageHeaderMetrics::TOOLBAR_BUTTON_SIZE)
            .accessibility_label(Some(tr("map_compass_reset")))
            .on_press(|_, cx| {
                MapInteractionsStore::shared(cx).update(cx, |store, cx| store.reset_map_bearing(cx));
            })
            .child(MapCompassNeedle { color })
    }
}

#[derive(IntoElement)]
struct MapCompassNeedle {
    color: Hsla,
}

impl RenderOnce for MapCompassNeedle {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let view =
            window.use_keyed_state(ElementId::Name("map-compass-needle".into()), cx, |_, _| MapCompassNeedleView::new());
        let link = MapGlobeCameraLink::shared(cx);
        if link.read(cx).compass().is_none_or(|compass| compass != view) {
            link.update(cx, |link, _| link.compass = Some(view.downgrade()));
        }
        view.update(cx, |view, cx| view.set_color(self.color, cx));
        link.update(cx, |link, cx| link.update_compass(cx));
        div().size(px(29.)).child(view.cached(StyleRefinement::default().size_full()))
    }
}

pub struct MapCompassNeedleView {
    color: Hsla,
    bearing: f64,
}

impl MapCompassNeedleView {
    fn new() -> Self {
        Self {
            color: Hsla::default(),
            bearing: f64::NAN,
        }
    }

    pub fn set_color(&mut self, color: Hsla, cx: &mut Context<Self>) {
        if self.color != color {
            self.color = color;
            cx.notify();
        }
    }

    pub fn set_bearing(&mut self, value: f64, cx: &mut Context<Self>) {
        if !value.is_finite() || value == self.bearing {
            return;
        }
        self.bearing = value;
        cx.notify();
    }

    fn paint_needle(bounds: Bounds<Pixels>, color: Hsla, bearing: f64, window: &mut Window) {
        let centre = bounds.center();
        let radius = f32::from(bounds.size.width.min(bounds.size.height)) / 2.0;

        let mut marks = PathBuilder::stroke(px(1.));
        for index in 0..4 {
            let angle = index as f32 * PI / 2.0;
            let (sin, cos) = angle.sin_cos();
            marks.move_to(point(centre.x + px(cos * (radius - 2.0)), centre.y + px(sin * (radius - 2.0))));
            marks.line_to(point(centre.x + px(cos * radius), centre.y + px(sin * radius)));
        }
        if let Ok(path) = marks.build() {
            window.paint_path(path, color.opacity(0.3));
        }

        let rotation = (-bearing * std::f64::consts::PI / 180.0) as f32;
        let (sin, cos) = rotation.sin_cos();
        let rotated = |x: f32, y: f32| point(centre.x + px(x * cos - y * sin), centre.y + px(x * sin + y * cos));
        for (sign, fill) in [(-1.0_f32, SYSTEM_RED), (1.0, color.opacity(0.65))] {
            let mut shape = PathBuilder::fill();
            shape.move_to(rotated(0.0, sign * (radius - 3.0)));
            shape.line_to(rotated(4.0, 0.0));
            shape.line_to(rotated(0.0, -sign * 1.5));
            shape.line_to(rotated(-4.0, 0.0));
            shape.close();
            if let Ok(path) = shape.build() {
                window.paint_path(path, fill);
            }
        }
    }
}

impl Render for MapCompassNeedleView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let color = self.color;
        let bearing = self.bearing;
        canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let bearing = if bearing.is_finite() { bearing } else { 0.0 };
                Self::paint_needle(bounds, color, bearing, window);
            },
        )
        .size_full()
    }
}
