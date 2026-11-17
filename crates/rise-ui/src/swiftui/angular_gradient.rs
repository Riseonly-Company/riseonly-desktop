use gpui::{App, Bounds, Hsla, IntoElement, Path, Pixels, Rgba, Styled, Window, canvas, point, px};

const MAX_SECTOR_ARC: f32 = 1.5;
const MIN_SECTORS: usize = 48;
const MAX_SECTORS: usize = 720;

const OPAQUE_SEAM_OVERLAP: f32 = 0.2;

/// angular gradient around the shape center, colors spread over the full turn
/// starts at angle degrees, 0 is trailing side and it grows clockwise
#[derive(Clone, Debug, PartialEq)]
pub struct AngularGradient {
    pub colors: Vec<Hsla>,
    pub angle: f32,
}

impl AngularGradient {
    /// gradient from these colors around the center
    pub fn new(colors: impl IntoIterator<Item = Hsla>) -> Self {
        Self {
            colors: colors.into_iter().collect(),
            angle: 0.0,
        }
    }

    /// turn starts at this many degrees
    pub fn angle(mut self, degrees: f32) -> Self {
        self.angle = degrees;
        self
    }

    /// color at this fraction of the turn, 0 and 1 are the same spot
    /// neighbours blend in srgb
    pub fn color_at(&self, fraction: f32) -> Hsla {
        match self.colors.as_slice() {
            [] => Hsla::transparent_black(),
            [only] => *only,
            colors => {
                let last = colors.len() - 1;
                let position = fraction.clamp(0.0, 1.0) * last as f32;
                let index = (position.floor() as usize).min(last - 1);
                let local = position - index as f32;
                mix_srgb(colors[index], colors[index + 1], local)
            }
        }
    }

    /// color shown in this screen direction
    pub fn color_at_angle(&self, degrees: f32) -> Hsla {
        self.color_at((degrees - self.angle).rem_euclid(360.0) / 360.0)
    }

    fn is_opaque(&self) -> bool {
        self.colors.iter().all(|color| color.a >= 1.0)
    }
}

fn mix_srgb(from: Hsla, to: Hsla, amount: f32) -> Hsla {
    let from = from.to_rgb();
    let to = to.to_rgb();
    let lerp = |a: f32, b: f32| a + (b - a) * amount;
    Hsla::from(Rgba {
        r: lerp(from.r, to.r),
        g: lerp(from.g, to.g),
        b: lerp(from.b, to.b),
        a: lerp(from.a, to.a),
    })
}

/// how many sectors a ring of this diameter is painted with
pub fn sector_count(diameter: f32) -> usize {
    let circumference = std::f32::consts::PI * diameter.max(0.0);
    ((circumference / MAX_SECTOR_ARC).ceil() as usize).clamp(MIN_SECTORS, MAX_SECTORS)
}

/// point on a circle at given degrees, 0 is right and it goes clockwise
pub fn point_on_circle(center: (f32, f32), radius: f32, degrees: f32) -> (f32, f32) {
    let radians = degrees.to_radians();
    (
        center.0 + radius * radians.cos(),
        center.1 + radius * radians.sin(),
    )
}

/// circle stroke with the gradient, centered on the circle inside the frame
/// so half the stroke sticks out of the frame
pub fn angular_gradient_circle_stroke(
    gradient: AngularGradient,
    line_width: f32,
    diameter: f32,
) -> impl IntoElement {
    canvas(
        |_: Bounds<Pixels>, _: &mut Window, _: &mut App| {},
        move |bounds: Bounds<Pixels>, (), window: &mut Window, _: &mut App| {
            paint_ring(&gradient, line_width, diameter, bounds, window);
        },
    )
    .w(px(diameter))
    .h(px(diameter))
}

fn paint_ring(
    gradient: &AngularGradient,
    line_width: f32,
    diameter: f32,
    bounds: Bounds<Pixels>,
    window: &mut Window,
) {
    if gradient.colors.is_empty() || line_width <= 0.0 || diameter <= 0.0 {
        return;
    }
    let center = (
        f32::from(bounds.origin.x) + f32::from(bounds.size.width) / 2.0,
        f32::from(bounds.origin.y) + f32::from(bounds.size.height) / 2.0,
    );
    let radius = diameter / 2.0;
    let inner = (radius - line_width / 2.0).max(0.0);
    let outer = radius + line_width / 2.0;
    let sectors = sector_count(diameter);
    let step = 360.0 / sectors as f32;
    let overlap = if gradient.is_opaque() {
        OPAQUE_SEAM_OVERLAP
    } else {
        0.0
    };

    for sector in 0..sectors {
        let start = gradient.angle + sector as f32 * step;
        let end = start + step + overlap;
        let color = gradient.color_at((sector as f32 + 0.5) / sectors as f32);
        let corners = [
            point_on_circle(center, inner, start),
            point_on_circle(center, outer, start),
            point_on_circle(center, outer, end),
            point_on_circle(center, inner, end),
        ];
        let mut path = Path::new(point(px(corners[0].0), px(corners[0].1)));
        for &(x, y) in &corners[1..] {
            path.line_to(point(px(x), px(y)));
        }
        window.paint_path(path, color);
    }
}
