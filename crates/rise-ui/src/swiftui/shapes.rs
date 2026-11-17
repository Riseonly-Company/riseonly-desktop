use gpui::{AnyElement, Div, Hsla, IntoElement, ParentElement, StyleRefinement, Styled, div, px};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum RoundedCornerStyle {
    #[default]
    Circular,
    Continuous,
}

/// shapes the app clips and strokes and hit tests with
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Rectangle,
    RoundedRectangle { corner_radius: f32, style: RoundedCornerStyle },
    Capsule,
    Circle,
}

impl Shape {
    pub const fn rounded_rect(corner_radius: f32) -> Self {
        Self::RoundedRectangle { corner_radius, style: RoundedCornerStyle::Circular }
    }

    /// rounded rectangle with continuous corners
    pub const fn continuous(corner_radius: f32) -> Self {
        Self::RoundedRectangle { corner_radius, style: RoundedCornerStyle::Continuous }
    }

    /// same shape grown by distance on every side, what the outer edge of a centered stroke follows
    pub(crate) fn outset(self, distance: f32) -> Self {
        match self {
            Self::Rectangle => Self::Rectangle,
            Self::RoundedRectangle { corner_radius, style } => {
                Self::RoundedRectangle { corner_radius: corner_radius + distance, style }
            }
            Self::Capsule => Self::Capsule,
            Self::Circle => Self::Circle,
        }
    }

    /// rounds the element like this shape
    pub fn round<E: Styled>(self, element: E) -> E {
        match self {
            Self::Rectangle => element,
            Self::RoundedRectangle { corner_radius, style: RoundedCornerStyle::Circular } => {
                element.rounded(px(corner_radius))
            }
            Self::RoundedRectangle { corner_radius, style: RoundedCornerStyle::Continuous } => {
                element.continuous_corners(corner_radius)
            }
            Self::Capsule | Self::Circle => element.rounded_full(),
        }
    }
}

/// shape modifiers that only style the element itself
pub trait SwiftUiShapes: Styled + Sized {
    /// capsule as the element shape
    fn capsule(self) -> Self {
        self.rounded_full()
    }

    /// circle as the element shape, square and then fully rounded
    fn circle(self) -> Self {
        self.rounded_full().aspect_ratio(1.)
    }

    /// rounded rect as the element own shape
    fn rounded_rect(self, corner_radius: f32) -> Self {
        self.rounded(px(corner_radius))
    }

    /// continuous rounded corners, circular arcs until the quad shader draws superellipses
    /// use this and never .rounded() so one switch reaches all of them
    fn continuous_corners(self, corner_radius: f32) -> Self {
        self.rounded(px(corner_radius))
    }

    fn clip_shape(self, shape: Shape) -> Self {
        shape.round(self).overflow_hidden()
    }

    /// border stroked on the element own outline, gpui borders are drawn inside the bounds
    /// round the element first
    fn stroke_border(self, color: impl Into<Hsla>, line_width: f32) -> Self {
        self.border(px(line_width)).border_color(color)
    }
}

impl<T: Styled> SwiftUiShapes for T {}

/// stroke centered on the outline, so half of it lies outside the bounds
/// add it as child of the stroked element, overflow_hidden parent clips outer half
/// clicks pass through to what is under
pub fn centred_stroke(color: impl Into<Hsla>, line_width: f32, shape: Shape) -> Div {
    let half = line_width / 2.;
    shape
        .outset(half)
        .round(div().absolute().inset(px(-half)))
        .border(px(line_width))
        .border_color(color)
}

/// transparent layer over the whole element that takes the tap
/// so empty space between children is tappable, add it as the last child with the handler
/// hit tests the bounding box so corners of a capsule or circle answer too
pub fn hit_area(shape: Shape) -> Div {
    shape.round(div().absolute().inset_0())
}

/// styled container that cant take handlers or block the mouse
/// clicks and hovers and scrolls go to whatever is under it
/// children with their own handler still answer, decorative content only
pub struct PassThrough {
    base: Div,
}

/// new pass through container
pub fn pass_through() -> PassThrough {
    PassThrough { base: div() }
}

impl Styled for PassThrough {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl ParentElement for PassThrough {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.base.extend(elements);
    }
}

impl IntoElement for PassThrough {
    type Element = Div;

    fn into_element(self) -> Self::Element {
        self.base
    }
}
