use std::f32::consts::FRAC_PI_2;
use std::sync::LazyLock;

use gpui::{
    App, BACKDROP_BLUR_SUPPORTED, Corners, Hsla, IntoElement, ParentElement, RenderOnce, Styled, Window, canvas,
    div, px, relative, white,
};

use crate::swiftui::environment_values::accessibility_reduce_transparency;
use crate::swiftui::linear_gradient::{GradientStop, LinearGradient};
use crate::swiftui::shapes::pass_through;
use crate::swiftui::transition::UnitPoint;

#[derive(IntoElement)]
pub struct ScrollEdgeBlurUi {
    pub max_radius: f32,
    pub solid_end: f32,
    pub tint_peak: f32,
    pub from_bottom: bool,
    pub fit_height: Option<f32>,
}

impl Default for ScrollEdgeBlurUi {
    fn default() -> Self {
        Self {
            max_radius: ScrollEdgeBlurFalloff::EDGE_RADIUS,
            solid_end: ScrollEdgeBlurFalloff::DEFAULT_SOLID_END,
            tint_peak: ScrollEdgeBlurFalloff::TINT_PEAK,
            from_bottom: false,
            fit_height: None,
        }
    }
}

impl ScrollEdgeBlurUi {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn max_radius(mut self, max_radius: f32) -> Self {
        self.max_radius = max_radius;
        self
    }

    pub fn solid_end(mut self, solid_end: f32) -> Self {
        self.solid_end = solid_end;
        self
    }

    pub fn tint_peak(mut self, tint_peak: f32) -> Self {
        self.tint_peak = tint_peak;
        self
    }

    pub fn from_bottom(mut self, from_bottom: bool) -> Self {
        self.from_bottom = from_bottom;
        self
    }

    /// layer exactly this tall from its edge, the band is not stretched past it
    pub fn fit_height(mut self, height: f32) -> Self {
        self.fit_height = Some(height.max(0.0));
        self
    }
}

impl RenderOnce for ScrollEdgeBlurUi {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let color = crate::theme(cx).bg._100;
        let reduce_transparency = accessibility_reduce_transparency(cx);
        let (scale, solid_end) = match self.fit_height {
            Some(_) => (1.0, self.solid_end.clamp(0.0, 1.0)),
            None => ScrollEdgeBlurFalloff::stretched(self.solid_end),
        };
        let stops = ScrollEdgeBlurFalloff::paint_stops(
            color,
            solid_end,
            self.tint_peak,
            reduce_transparency,
            BACKDROP_BLUR_SUPPORTED,
        );
        let from_bottom = self.from_bottom;
        let blur = (BACKDROP_BLUR_SUPPORTED && !reduce_transparency && self.max_radius > 0.0).then(|| {
            let sigma = px(self.max_radius);
            div().absolute().top_0().left_0().size_full().child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        window.paint_progressive_backdrop_blur(bounds, Corners::default(), sigma, px(0.0), from_bottom);
                    },
                )
                .size_full(),
            )
        });
        let (start, end) = if from_bottom { (UnitPoint::BOTTOM, UnitPoint::TOP) } else { (UnitPoint::TOP, UnitPoint::BOTTOM) };
        let layer = pass_through().absolute().left_0().right_0();
        let layer = match self.fit_height {
            Some(height) => layer.h(px(height)),
            None => layer.h(relative(scale)),
        };
        let layer = if from_bottom { layer.bottom_0() } else { layer.top_0() };
        pass_through()
            .relative()
            .size_full()
            .child(layer.children(blur).child(LinearGradient::new(stops, start, end)))
    }
}

pub struct ScrollEdgeBlurFalloff;

static MASK_IMAGE: LazyLock<Vec<GradientStop>> =
    LazyLock::new(|| ScrollEdgeBlurFalloff::mask_image(white(), ScrollEdgeBlurFalloff::DEFAULT_SOLID_END));

impl ScrollEdgeBlurFalloff {
    pub const DEFAULT_SOLID_END: f32 = 0.32;
    pub const EDGE_RADIUS: f32 = 20.0;
    pub const TINT_PEAK: f32 = 0.45;
    pub const HEADER_TINT_PEAK: f32 = 0.88;
    pub const HEADER_TINT_PEAK_DARK: f32 = 0.96;
    pub const HEADER_BLURRED_TINT_PEAK: f32 = 0.5;
    pub const HEADER_BLURRED_TINT_PEAK_DARK: f32 = 0.58;
    pub const BAND_STRETCH: f32 = 2.0;

    /// height and solid_end of a blur held behind a header and fading out over fade below it
    pub fn header_band(header_height: f32, fade: f32) -> (f32, f32) {
        let header = header_height.max(0.0);
        let height = header + fade.max(0.0);
        if height <= 0.0 {
            return (0.0, 0.0);
        }
        (height, header / height)
    }

    /// layer height as a share of the given one and solid_end inside it
    /// the band under solid_end runs BAND_STRETCH times longer past the given bottom
    pub fn stretched(solid_end: f32) -> (f32, f32) {
        let solid_end = solid_end.clamp(0.0, 1.0);
        let scale = solid_end + (1.0 - solid_end) * Self::BAND_STRETCH;
        (scale, solid_end / scale)
    }

    /// header tint for the page appearance
    /// lighter over the real blur and held near opaque where the renderer has none
    pub fn header_tint_peak(dark: bool) -> f32 {
        match (BACKDROP_BLUR_SUPPORTED, dark) {
            (true, true) => Self::HEADER_BLURRED_TINT_PEAK_DARK,
            (true, false) => Self::HEADER_BLURRED_TINT_PEAK,
            (false, true) => Self::HEADER_TINT_PEAK_DARK,
            (false, false) => Self::HEADER_TINT_PEAK,
        }
    }
    const ROWS: usize = 64;

    /// what the scroll edge blur paints in color, three stop gradient when transparency is reduced
    /// else the mask curve with the tint held at TINT_PEAK
    pub fn tint_stops(color: Hsla, solid_end: f32, reduce_transparency: bool) -> Vec<GradientStop> {
        Self::tint_stops_with_peak(color, solid_end, Self::TINT_PEAK, reduce_transparency)
    }

    /// tint_stops with the tint held at tint_peak instead of TINT_PEAK
    pub fn tint_stops_with_peak(
        color: Hsla,
        solid_end: f32,
        tint_peak: f32,
        reduce_transparency: bool,
    ) -> Vec<GradientStop> {
        if reduce_transparency {
            return vec![
                GradientStop::new(color, 0.0),
                GradientStop::new(color, solid_end),
                GradientStop::new(color.opacity(0.0), 1.0),
            ];
        }
        Self::mask_image(Hsla { a: color.a * tint_peak, ..color }, solid_end)
    }

    /// what the layer paints, the tint over a real blur or an opaque falloff where the renderer cannot blur
    /// the opaque edge thins out over the same band the blur would cover, so nothing under it shows through
    pub fn paint_stops(
        color: Hsla,
        solid_end: f32,
        tint_peak: f32,
        reduce_transparency: bool,
        renderer_blurs: bool,
    ) -> Vec<GradientStop> {
        if renderer_blurs || reduce_transparency {
            return Self::tint_stops_with_peak(color, solid_end, tint_peak, reduce_transparency);
        }
        Self::mask_image(color, solid_end)
    }

    pub fn mask_image_default() -> &'static [GradientStop] {
        &MASK_IMAGE
    }

    /// blur strength as alpha painted in tint
    /// one stop per row of the 1 by 64 mask at the row center, top first
    /// before the first and after the last center the gradient holds the edge row
    pub fn mask_image(tint: Hsla, solid_end: f32) -> Vec<GradientStop> {
        let rows = Self::ROWS as f32;
        (0..Self::ROWS)
            .map(|row| {
                let from_top = (row as f32 + 0.5) / rows;
                let strength = Self::strength(from_top, solid_end);
                GradientStop::new(Hsla { a: tint.a * strength, ..tint }, from_top)
            })
            .collect()
    }

    /// strength at from_top, a share of the height measured from the top edge
    /// full down to solid_end, then an ease in seen from the content side, the same curve as the blur radius
    pub fn strength(from_top: f32, solid_end: f32) -> f32 {
        let t = ((from_top - solid_end) / (1.0 - solid_end).max(f32::EPSILON)).clamp(0.0, 1.0);
        1.0 - ((1.0 - t) * FRAC_PI_2).cos()
    }

    /// the white mask, shared cached one at the default falloff and a fresh one otherwise
    pub fn image(solid_end: f32) -> Vec<GradientStop> {
        if solid_end == Self::DEFAULT_SOLID_END {
            MASK_IMAGE.clone()
        } else {
            Self::mask_image(white(), solid_end)
        }
    }
}
