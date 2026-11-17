use std::f64::consts::PI;

use gpui::{Background, Bounds, Path, PathBuilder, Pixels, Point, Window, point, px, size};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(i64)]
pub enum MessageBubbleTailSide {
    #[default]
    None = 0,
    Leading = 1,
    Trailing = 2,
}

impl MessageBubbleTailSide {
    pub const fn raw_value(self) -> i64 {
        self as i64
    }

    pub const fn from_raw(raw: i64) -> Option<Self> {
        match raw {
            0 => Some(Self::None),
            1 => Some(Self::Leading),
            2 => Some(Self::Trailing),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MessageBubbleRadii {
    pub top_leading: Pixels,
    pub top_trailing: Pixels,
    pub bottom_leading: Pixels,
    pub bottom_trailing: Pixels,
}

impl MessageBubbleRadii {
    pub const fn uniform(radius: Pixels) -> Self {
        Self {
            top_leading: radius,
            top_trailing: radius,
            bottom_leading: radius,
            bottom_trailing: radius,
        }
    }

    fn horizontally_mirrored(self) -> Self {
        Self {
            top_leading: self.top_trailing,
            top_trailing: self.top_leading,
            bottom_leading: self.bottom_trailing,
            bottom_trailing: self.bottom_leading,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MessageBubbleDescriptor {
    pub radii: MessageBubbleRadii,
    pub tail_side: MessageBubbleTailSide,
    pub draws_tail: bool,
}

impl MessageBubbleDescriptor {
    pub fn grouped(
        is_outgoing: bool,
        is_first: bool,
        is_last: bool,
        corner_radius: Pixels,
        merged_corner_radius: Pixels,
    ) -> Self {
        if is_outgoing {
            return Self {
                radii: MessageBubbleRadii {
                    top_leading: corner_radius,
                    top_trailing: if is_first { corner_radius } else { merged_corner_radius },
                    bottom_leading: corner_radius,
                    bottom_trailing: if is_last { corner_radius } else { merged_corner_radius },
                },
                tail_side: MessageBubbleTailSide::Trailing,
                draws_tail: is_last,
            };
        }
        Self {
            radii: MessageBubbleRadii {
                top_leading: if is_first { corner_radius } else { merged_corner_radius },
                top_trailing: corner_radius,
                bottom_leading: if is_last { corner_radius } else { merged_corner_radius },
                bottom_trailing: corner_radius,
            },
            tail_side: MessageBubbleTailSide::Leading,
            draws_tail: is_last,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MessageBubblePathElement {
    MoveTo(Point<Pixels>),
    LineTo(Point<Pixels>),
    QuadCurveTo { to: Point<Pixels>, control: Point<Pixels> },
    CurveTo { to: Point<Pixels>, control1: Point<Pixels>, control2: Point<Pixels> },
    CloseSubpath,
}

/// the contour the geometry builds, resolution free
/// native cells, lifted previews and gpui painting all read it so the bubble comes from one contract
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MessageBubblePath {
    elements: Vec<MessageBubblePathElement>,
    start: Option<Point<Pixels>>,
    current: Option<Point<Pixels>>,
}

impl MessageBubblePath {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn elements(&self) -> &[MessageBubblePathElement] {
        &self.elements
    }

    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    pub fn current_point(&self) -> Point<Pixels> {
        self.current.unwrap_or_default()
    }

    pub fn move_to(&mut self, to: Point<Pixels>) {
        self.elements.push(MessageBubblePathElement::MoveTo(to));
        self.start = Some(to);
        self.current = Some(to);
    }

    pub fn add_line(&mut self, to: Point<Pixels>) {
        self.elements.push(MessageBubblePathElement::LineTo(to));
        self.current = Some(to);
    }

    pub fn add_quad_curve(&mut self, to: Point<Pixels>, control: Point<Pixels>) {
        self.elements
            .push(MessageBubblePathElement::QuadCurveTo { to, control });
        self.current = Some(to);
    }

    pub fn add_curve(&mut self, to: Point<Pixels>, control1: Point<Pixels>, control2: Point<Pixels>) {
        self.elements.push(MessageBubblePathElement::CurveTo {
            to,
            control1,
            control2,
        });
        self.current = Some(to);
    }

    pub fn close_subpath(&mut self) {
        self.elements.push(MessageBubblePathElement::CloseSubpath);
        self.current = self.start;
    }

    pub fn horizontally_mirrored(&self, axis_sum: Pixels) -> Self {
        self.mapped(|p| point(axis_sum - p.x, p.y))
    }

    /// copy with an axis aligned transform, x becomes a x plus tx and y becomes d y plus ty
    pub fn scaled_and_translated(&self, a: f32, d: f32, tx: Pixels, ty: Pixels) -> Self {
        self.mapped(|p| point(p.x * a + tx, p.y * d + ty))
    }

    /// copy with every point of the contour passed through mirror
    pub fn mapped(&self, mirror: impl Fn(Point<Pixels>) -> Point<Pixels>) -> Self {
        Self {
            elements: self
                .elements
                .iter()
                .map(|element| match *element {
                    MessageBubblePathElement::MoveTo(p) => MessageBubblePathElement::MoveTo(mirror(p)),
                    MessageBubblePathElement::LineTo(p) => MessageBubblePathElement::LineTo(mirror(p)),
                    MessageBubblePathElement::QuadCurveTo { to, control } => {
                        MessageBubblePathElement::QuadCurveTo {
                            to: mirror(to),
                            control: mirror(control),
                        }
                    }
                    MessageBubblePathElement::CurveTo {
                        to,
                        control1,
                        control2,
                    } => MessageBubblePathElement::CurveTo {
                        to: mirror(to),
                        control1: mirror(control1),
                        control2: mirror(control2),
                    },
                    MessageBubblePathElement::CloseSubpath => MessageBubblePathElement::CloseSubpath,
                })
                .collect(),
            start: self.start.map(&mirror),
            current: self.current.map(&mirror),
        }
    }

    /// replays the contour into a gpui builder, caller picks fill or stroke
    pub fn replay_into(&self, builder: &mut PathBuilder) {
        for element in &self.elements {
            match *element {
                MessageBubblePathElement::MoveTo(p) => builder.move_to(p),
                MessageBubblePathElement::LineTo(p) => builder.line_to(p),
                MessageBubblePathElement::QuadCurveTo { to, control } => builder.curve_to(to, control),
                MessageBubblePathElement::CurveTo {
                    to,
                    control1,
                    control2,
                } => builder.cubic_bezier_to(to, control1, control2),
                MessageBubblePathElement::CloseSubpath => builder.close(),
            }
        }
    }

    /// filled gpui path, none for an empty contour or one the tessellator refuses
    pub fn fill_path(&self) -> Option<Path<Pixels>> {
        if self.is_empty() {
            return None;
        }
        let mut builder = PathBuilder::fill();
        self.replay_into(&mut builder);
        builder.build().ok()
    }
}

/// one geometry contract for bubbles, native text cells and lifted previews
/// terminal contour is a clean room vector rebuild of Telegram 33 point body with a 6 point
/// tail allowance
pub enum MessageBubbleGeometry {}

impl MessageBubbleGeometry {
    pub const TAIL_INSET: Pixels = px(6.0);

    pub fn body_rect(bounds: Bounds<Pixels>, tail_side: MessageBubbleTailSide) -> Bounds<Pixels> {
        match tail_side {
            MessageBubbleTailSide::None => bounds,
            MessageBubbleTailSide::Leading => Bounds::new(
                point(bounds.left() + Self::TAIL_INSET, bounds.top()),
                size(
                    (bounds.size.width - Self::TAIL_INSET).max(px(0.0)),
                    bounds.size.height,
                ),
            ),
            MessageBubbleTailSide::Trailing => Bounds::new(
                point(bounds.left(), bounds.top()),
                size(
                    (bounds.size.width - Self::TAIL_INSET).max(px(0.0)),
                    bounds.size.height,
                ),
            ),
        }
    }

    pub fn path(
        bounds: Bounds<Pixels>,
        radii: MessageBubbleRadii,
        tail_side: MessageBubbleTailSide,
        draws_tail: bool,
    ) -> MessageBubblePath {
        if !(bounds.size.width > px(0.0) && bounds.size.height > px(0.0)) {
            return MessageBubblePath::new();
        }

        match tail_side {
            MessageBubbleTailSide::None => Self::rounded_body_path(bounds, radii),
            MessageBubbleTailSide::Trailing => Self::outgoing_path(bounds, radii, draws_tail),
            MessageBubbleTailSide::Leading => {
                let outgoing = Self::outgoing_path(bounds, radii.horizontally_mirrored(), draws_tail);
                outgoing.horizontally_mirrored(bounds.left() + bounds.right())
            }
        }
    }

    fn outgoing_path(bounds: Bounds<Pixels>, radii: MessageBubbleRadii, draws_tail: bool) -> MessageBubblePath {
        let body = Self::body_rect(bounds, MessageBubbleTailSide::Trailing);
        let body_width = px_f64(body.size.width);
        let body_height = px_f64(body.size.height);
        if !(draws_tail
            && body_width >= 33.0
            && body_height >= 33.0
            && px_f64(radii.bottom_trailing) >= 16.0)
        {
            return Self::rounded_body_path(body, radii);
        }

        let max_radius = body_width.min(body_height) * 0.5;
        let top_leading = clamped(px_f64(radii.top_leading), max_radius);
        let top_trailing = clamped(px_f64(radii.top_trailing), max_radius);
        let bottom_leading = clamped(px_f64(radii.bottom_leading), max_radius);
        let bottom_trailing = 16.0_f64.min(clamped(px_f64(radii.bottom_trailing), max_radius));

        let left = px_f64(body.left());
        let right = px_f64(body.right());
        let top = px_f64(body.top());
        let bottom = px_f64(body.bottom());
        let mut path = MessageBubblePath::new();

        path.move_to(pt(left + top_leading, top));
        path.add_line(pt(right - top_trailing, top));
        add_elliptical_arc(
            &mut path,
            (right - top_trailing, top + top_trailing),
            top_trailing,
            top_trailing,
            -PI / 2.0,
            0.0,
        );

        path.add_line(pt(right, bottom - 8.5));

        add_elliptical_arc(
            &mut path,
            (right + 11.5, bottom - 8.5),
            11.5,
            10.5,
            PI,
            2.211_233_069_013_255,
        );

        path.add_quad_curve(pt(right + 4.5, bottom), pt(right + 4.564_265_226_189_312, bottom));
        path.add_quad_curve(
            pt(right - 6.730_470_791_433_198_5, bottom - 2.958_687_633_082_056),
            pt(right - 3.464_781_457_281_761_2, bottom),
        );

        add_elliptical_arc(
            &mut path,
            (right - bottom_trailing, bottom - bottom_trailing),
            bottom_trailing,
            bottom_trailing,
            0.952_870_759_437_246_5,
            PI / 2.0,
        );
        path.add_line(pt(left + bottom_leading, bottom));
        add_elliptical_arc(
            &mut path,
            (left + bottom_leading, bottom - bottom_leading),
            bottom_leading,
            bottom_leading,
            PI / 2.0,
            PI,
        );
        path.add_line(pt(left, top + top_leading));
        add_elliptical_arc(
            &mut path,
            (left + top_leading, top + top_leading),
            top_leading,
            top_leading,
            PI,
            PI * 1.5,
        );
        path.close_subpath();
        path
    }

    fn rounded_body_path(rect: Bounds<Pixels>, radii: MessageBubbleRadii) -> MessageBubblePath {
        let width = px_f64(rect.size.width);
        let height = px_f64(rect.size.height);
        if !(width > 0.0 && height > 0.0) {
            return MessageBubblePath::new();
        }
        let maximum_radius = width.min(height) * 0.5;
        let top_leading = clamped(px_f64(radii.top_leading), maximum_radius);
        let top_trailing = clamped(px_f64(radii.top_trailing), maximum_radius);
        let bottom_leading = clamped(px_f64(radii.bottom_leading), maximum_radius);
        let bottom_trailing = clamped(px_f64(radii.bottom_trailing), maximum_radius);
        let min_x = px_f64(rect.left());
        let max_x = px_f64(rect.right());
        let min_y = px_f64(rect.top());
        let max_y = px_f64(rect.bottom());
        let mut path = MessageBubblePath::new();

        path.move_to(pt(min_x + top_leading, min_y));
        path.add_line(pt(max_x - top_trailing, min_y));
        add_elliptical_arc(
            &mut path,
            (max_x - top_trailing, min_y + top_trailing),
            top_trailing,
            top_trailing,
            -PI / 2.0,
            0.0,
        );
        path.add_line(pt(max_x, max_y - bottom_trailing));
        add_elliptical_arc(
            &mut path,
            (max_x - bottom_trailing, max_y - bottom_trailing),
            bottom_trailing,
            bottom_trailing,
            0.0,
            PI / 2.0,
        );
        path.add_line(pt(min_x + bottom_leading, max_y));
        add_elliptical_arc(
            &mut path,
            (min_x + bottom_leading, max_y - bottom_leading),
            bottom_leading,
            bottom_leading,
            PI / 2.0,
            PI,
        );
        path.add_line(pt(min_x, min_y + top_leading));
        add_elliptical_arc(
            &mut path,
            (min_x + top_leading, min_y + top_leading),
            top_leading,
            top_leading,
            PI,
            PI * 1.5,
        );
        path.close_subpath();
        path
    }
}

fn pt(x: f64, y: f64) -> Point<Pixels> {
    point(px(x as f32), px(y as f32))
}

fn px_f64(value: Pixels) -> f64 {
    f64::from(f32::from(value))
}

fn add_elliptical_arc(
    path: &mut MessageBubblePath,
    center: (f64, f64),
    radius_x: f64,
    radius_y: f64,
    start_angle: f64,
    end_angle: f64,
) {
    let (cx, cy) = center;
    if !(radius_x > 0.0 && radius_y > 0.0) {
        path.add_line(pt(
            cx + end_angle.cos() * radius_x,
            cy + end_angle.sin() * radius_y,
        ));
        return;
    }

    let delta = end_angle - start_angle;
    let segment_count = ((delta.abs() / (PI / 2.0)).ceil() as usize).max(1);
    let segment_delta = delta / segment_count as f64;

    for index in 0..segment_count {
        let a0 = start_angle + index as f64 * segment_delta;
        let a1 = a0 + segment_delta;
        let tangent_scale = 4.0 / 3.0 * ((a1 - a0) / 4.0).tan();
        let p0 = (cx + a0.cos() * radius_x, cy + a0.sin() * radius_y);
        let p1 = (cx + a1.cos() * radius_x, cy + a1.sin() * radius_y);
        if index == 0 && path.current_point() != pt(p0.0, p0.1) {
            path.add_line(pt(p0.0, p0.1));
        }
        path.add_curve(
            pt(p1.0, p1.1),
            pt(
                p0.0 - a0.sin() * radius_x * tangent_scale,
                p0.1 + a0.cos() * radius_y * tangent_scale,
            ),
            pt(
                p1.0 + a1.sin() * radius_x * tangent_scale,
                p1.1 - a1.cos() * radius_y * tangent_scale,
            ),
        );
    }
}

fn clamped(radius: f64, maximum: f64) -> f64 {
    radius.max(0.0).min(maximum)
}

/// shape of the bubble, same contour for any frame
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MessageBubbleShape {
    pub radii: MessageBubbleRadii,
    pub tail_side: MessageBubbleTailSide,
    pub draws_tail: bool,
}

impl MessageBubbleShape {
    pub fn path(&self, rect: Bounds<Pixels>) -> MessageBubblePath {
        MessageBubbleGeometry::path(rect, self.radii, self.tail_side, self.draws_tail)
    }

    /// fills the shape in rect during the paint phase
    pub fn paint(&self, rect: Bounds<Pixels>, fill: impl Into<Background>, window: &mut Window) {
        if let Some(path) = self.path(rect).fill_path() {
            window.paint_path(path, fill);
        }
    }
}
