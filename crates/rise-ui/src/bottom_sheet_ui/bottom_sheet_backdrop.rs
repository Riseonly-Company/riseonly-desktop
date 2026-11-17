use std::rc::Rc;
use std::time::Instant;

use gpui::{
    AnyElement, App, BACKDROP_BLUR_SUPPORTED, Corners, Hsla, IntoElement, Pixels, Point, RenderOnce, Window, black,
    canvas,
};

use crate::bottom_sheet_ui::bottom_sheet_metrics::BottomSheetMetrics;
use crate::swiftui::animated::Animated;
use crate::swiftui::environment_values::accessibility_reduce_transparency;
use crate::swiftui::motion::ease_out;
use crate::swiftui::prelude::*;

type BackdropTap = Rc<dyn Fn(Point<Pixels>, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct BottomSheetBackdrop {
    pub visible: bool,
    pub opacity: f64,
    pub color: Hsla,
    pub blur_radius: f32,
    on_tap: BackdropTap,
}

impl BottomSheetBackdrop {
    pub fn new(
        visible: bool,
        on_tap: impl Fn(Point<Pixels>, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            visible,
            opacity: BottomSheetMetrics::BACKDROP_OPACITY,
            color: black(),
            blur_radius: 0.0,
            on_tap: Rc::new(on_tap),
        }
    }

    pub fn opacity(mut self, opacity: f64) -> Self {
        self.opacity = opacity;
        self
    }

    pub fn color(mut self, color: Hsla) -> Self {
        self.color = color;
        self
    }

    pub fn blur_radius(mut self, blur_radius: f32) -> Self {
        self.blur_radius = blur_radius;
        self
    }
}

/// layer that blurs everything under its bounds with a gaussian of sigma, once per paint
/// paint the dim after it, None where no backdrop blur or the user reduces transparency
pub fn backdrop_blur_layer(sigma: f32, cx: &App) -> Option<AnyElement> {
    if !BACKDROP_BLUR_SUPPORTED || sigma <= 0.0 || accessibility_reduce_transparency(cx) {
        return None;
    }
    let sigma = px(sigma);
    Some(
        div()
            .absolute()
            .inset_0()
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| window.paint_backdrop_blur(bounds, Corners::default(), sigma, px(0.0)),
                )
                .size_full(),
            )
            .into_any_element(),
    )
}

struct BackdropFade {
    origin: Instant,
    opacity: Animated<f32>,
}

impl RenderOnce for BottomSheetBackdrop {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let target = if self.visible { self.opacity as f32 } else { 0.0 };
        let clock = cx.background_executor().now();
        let fade = window.use_keyed_state("bottom-sheet-backdrop", cx, |_, _| BackdropFade {
            origin: clock,
            opacity: Animated::new(target),
        });
        let now = clock.saturating_duration_since(fade.read(cx).origin);
        fade.update(cx, |fade, cx| {
            fade.opacity
                .set_in(target, ease_out(BottomSheetMetrics::BACKDROP_FADE_DURATION), now, cx);
        });
        let opacity = fade.read(cx).opacity.frame_value(now, window);

        let fade_share = if self.opacity > 0.0 { (opacity / self.opacity as f32).clamp(0.0, 1.0) } else { 0.0 };
        let blur = backdrop_blur_layer(self.blur_radius * fade_share, cx);
        let layer = div()
            .absolute()
            .inset_0()
            .children(blur)
            .child(div().absolute().inset_0().bg(self.color.opacity(opacity)));
        if !self.visible {
            return layer.into_any_element();
        }
        let on_tap = self.on_tap;
        layer
            .id("bottom-sheet-backdrop-hit")
            .occlude()
            .on_click(move |event, window, cx| on_tap(event.position(), window, cx))
            .into_any_element()
    }
}
