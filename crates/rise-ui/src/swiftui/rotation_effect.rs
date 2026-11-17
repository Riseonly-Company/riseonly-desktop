use gpui::{
    AnyElement, App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, Pixels, Point, Radians, Size, TransformationMatrix, Window, size,
};

pub trait RotationEffectModifiers: IntoElement + Sized {
    /// turns the painted content by degrees around the frame center, clockwise when positive
    /// layout doesnt change
    fn rotation_effect(self, degrees: f32) -> RotationEffect {
        RotationEffect {
            child: self.into_any_element(),
            degrees,
            stretch: size(1.0, 1.0),
        }
    }
}

impl<T: IntoElement> RotationEffectModifiers for T {}

/// element from rotation_effect, lays the child out unchanged and paints it turned
pub struct RotationEffect {
    child: AnyElement,
    degrees: f32,
    stretch: Size<f32>,
}

impl RotationEffect {
    pub fn stretched(mut self, x: f32, y: f32) -> Self {
        self.stretch = size(x, y);
        self
    }
}

/// matrix in scaled window pixels that turns content clockwise by degrees around center
pub fn rotation_effect_matrix(
    center: Point<Pixels>,
    degrees: f32,
    scale_factor: f32,
) -> TransformationMatrix {
    TransformationMatrix::unit()
        .translate(center.scale(scale_factor))
        .rotate(Radians(degrees.to_radians()))
        .translate(center.scale(-scale_factor))
}

impl IntoElement for RotationEffect {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for RotationEffect {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let is_stretched = self.stretch.width != 1.0 || self.stretch.height != 1.0;
        if self.degrees.rem_euclid(360.0) == 0.0 && !is_stretched {
            self.child.paint(window, cx);
            return;
        }
        let scale_factor = window.scale_factor();
        let center = bounds.center();
        let matrix = if is_stretched {
            TransformationMatrix::unit()
                .translate(center.scale(scale_factor))
                .rotate(Radians(self.degrees.to_radians()))
                .scale(self.stretch)
                .translate(center.scale(-scale_factor))
        } else {
            rotation_effect_matrix(center, self.degrees, scale_factor)
        };
        window.with_transformation(matrix, |window| self.child.paint(window, cx));
    }
}
