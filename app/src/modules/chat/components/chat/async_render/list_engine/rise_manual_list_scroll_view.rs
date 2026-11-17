use gpui::{Bounds, Edges, Pixels, Point, Size};

/// enforces optional overscroll limits inside the offset setter
/// reverting in the scroll callback is too late, the bad offset already hit the screen
/// all offset writes go through set_content_offset
#[derive(Clone, Debug, PartialEq)]
pub struct RiseManualListScrollView {
    pub content_offset: Point<Pixels>,
    pub content_inset: Edges<Pixels>,
    pub content_size: Size<Pixels>,
    pub frame: Bounds<Pixels>,
    pub is_scroll_enabled: bool,
    pub is_tracking: bool,
    pub is_dragging: bool,
    pub is_decelerating: bool,
    pub bottom_overscroll_limit_y: Option<Pixels>,
    pub top_overscroll_limit_y: Option<Pixels>,
    pub cancels_control_touches: bool,
}

impl Default for RiseManualListScrollView {
    fn default() -> Self {
        Self {
            content_offset: Point::default(),
            content_inset: Edges::default(),
            content_size: Size::default(),
            frame: Bounds::default(),
            is_scroll_enabled: true,
            is_tracking: false,
            is_dragging: false,
            is_decelerating: false,
            bottom_overscroll_limit_y: None,
            top_overscroll_limit_y: None,
            cancels_control_touches: false,
        }
    }
}

impl RiseManualListScrollView {
    pub fn new() -> Self {
        Self::default()
    }

    /// visible rectangle in content coordinates
    pub fn bounds(&self) -> Bounds<Pixels> {
        Bounds::new(self.content_offset, self.frame.size)
    }

    pub fn touches_should_cancel(&self, view_is_control: bool) -> bool {
        if self.cancels_control_touches && view_is_control {
            return true;
        }
        !view_is_control
    }

    pub fn set_content_offset(&mut self, content_offset: Point<Pixels>) {
        self.content_offset = self.clamped(content_offset);
    }

    /// a jump lands at once, an animated write hands back the clamped target for the list to ride toward
    /// each frame of the ride goes through set_content_offset again
    pub fn set_content_offset_animated(&mut self, content_offset: Point<Pixels>, animated: bool) -> Point<Pixels> {
        let target = self.clamped(content_offset);
        if !animated {
            self.content_offset = target;
        }
        target
    }

    fn clamped(&self, offset: Point<Pixels>) -> Point<Pixels> {
        let mut result = offset;
        if let Some(limit) = self.top_overscroll_limit_y
            && limit.as_f32().is_finite()
        {
            result.y = result.y.max(limit);
        }
        if let Some(limit) = self.bottom_overscroll_limit_y
            && limit.as_f32().is_finite()
        {
            result.y = result.y.min(limit);
        }
        result
    }
}
