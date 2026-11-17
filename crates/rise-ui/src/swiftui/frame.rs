use std::ops::BitOr;

use gpui::{
    AnyElement, App, Div, InteractiveElement, Interactivity, IntoElement, ParentElement,
    RenderOnce, StyleRefinement, Styled, Window, div, px,
};

use crate::swiftui::environment::LayoutDirection;
use crate::swiftui::stacks::{Alignment, align_single_child};

pub const DEFAULT_PADDING: f32 = 16.0;

/// set of edges, like horizontal or top and leading
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct EdgeSet {
    pub top: bool,
    pub leading: bool,
    pub bottom: bool,
    pub trailing: bool,
}

impl EdgeSet {
    pub const TOP: Self = Self { top: true, leading: false, bottom: false, trailing: false };
    pub const LEADING: Self = Self { top: false, leading: true, bottom: false, trailing: false };
    pub const BOTTOM: Self = Self { top: false, leading: false, bottom: true, trailing: false };
    pub const TRAILING: Self = Self { top: false, leading: false, bottom: false, trailing: true };
    pub const HORIZONTAL: Self = Self { top: false, leading: true, bottom: false, trailing: true };
    pub const VERTICAL: Self = Self { top: true, leading: false, bottom: true, trailing: false };
    pub const ALL: Self = Self { top: true, leading: true, bottom: true, trailing: true };
}

impl BitOr for EdgeSet {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        Self {
            top: self.top || other.top,
            leading: self.leading || other.leading,
            bottom: self.bottom || other.bottom,
            trailing: self.trailing || other.trailing,
        }
    }
}

/// frame and padding bits that only size or pad the element itself
pub trait SwiftUiFrame: Styled + Sized {
    /// frame width and height with no alignment, none leaves that axis to the content
    /// the content keeps its placement, use the aligned frame when it must sit at an alignment
    fn frame_size(self, width: impl Into<Option<f32>>, height: impl Into<Option<f32>>) -> Self {
        let sized = match width.into() {
            Some(width) => self.w(px(width)),
            None => self,
        };
        match height.into() {
            Some(height) => sized.h(px(height)),
            None => sized,
        }
    }

    fn frame_min(self, width: impl Into<Option<f32>>, height: impl Into<Option<f32>>) -> Self {
        let sized = match width.into() {
            Some(width) => self.min_w(px(width)),
            None => self,
        };
        match height.into() {
            Some(height) => sized.min_h(px(height)),
            None => sized,
        }
    }

    fn padding_default(self) -> Self {
        self.p(px(DEFAULT_PADDING))
    }

    /// padding on given edges, leading and trailing follow the layout direction
    fn padding_edges(self, edges: EdgeSet, length: f32) -> Self {
        let rtl = LayoutDirection::current().is_right_to_left();
        let (left, right) =
            if rtl { (edges.trailing, edges.leading) } else { (edges.leading, edges.trailing) };
        let value = px(length);
        let padded = if edges.top { self.pt(value) } else { self };
        let padded = if edges.bottom { padded.pb(value) } else { padded };
        let padded = if left { padded.pl(value) } else { padded };
        if right { padded.pr(value) } else { padded }
    }
}

impl<T: Styled> SwiftUiFrame for T {}

/// frames that place their content at an alignment
/// real box around the element, inner stack keeps its alignment
/// max width wont widen a hugging stack
pub trait SwiftUiAlignedFrame: IntoElement + Sized {
    fn frame_max_width(self, alignment: Alignment) -> Frame {
        Frame::new(self, alignment).w_full()
    }

    fn frame_max_height(self, alignment: Alignment) -> Frame {
        Frame::new(self, alignment).h_full()
    }

    /// max width and height frame with alignment
    fn frame_max(self, alignment: Alignment) -> Frame {
        Frame::new(self, alignment).size_full()
    }

    /// aligned frame with width and height, none leaves that axis to the content
    fn frame_size_aligned(
        self,
        width: impl Into<Option<f32>>,
        height: impl Into<Option<f32>>,
        alignment: Alignment,
    ) -> Frame {
        Frame::new(self, alignment).frame_size(width, height)
    }
}

impl<T: IntoElement> SwiftUiAlignedFrame for T {}

/// box of an aligned frame, styles chained after it style the box so a background fills the frame
#[derive(IntoElement)]
pub struct Frame {
    alignment: Alignment,
    direction: LayoutDirection,
    base: Div,
    content: AnyElement,
}

impl Frame {
    pub fn new(content: impl IntoElement, alignment: Alignment) -> Self {
        Self {
            alignment,
            direction: LayoutDirection::current(),
            base: div(),
            content: content.into_any_element(),
        }
    }
}

impl Styled for Frame {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl InteractiveElement for Frame {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl RenderOnce for Frame {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        align_single_child(self.base, self.alignment, self.direction).child(self.content)
    }
}
