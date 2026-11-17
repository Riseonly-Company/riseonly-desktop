use std::cell::Cell;

use gpui::{
    AnyElement, App, Div, FlexDirection, InteractiveElement, Interactivity, IntoElement,
    Length, ParentElement, RenderOnce, StyleRefinement, Styled, Window, div, px,
};
use smallvec::SmallVec;

use crate::swiftui::environment::LayoutDirection;

pub const DEFAULT_STACK_SPACING: f32 = 8.0;

thread_local! {
    static LAYOUT_DIRECTION: Cell<LayoutDirection> = const { Cell::new(LayoutDirection::LeftToRight) };
}

impl LayoutDirection {
    /// direction the language store installed last
    pub fn current() -> Self {
        LAYOUT_DIRECTION.with(Cell::get)
    }

    pub fn install(direction: Self) {
        LAYOUT_DIRECTION.with(|cell| cell.set(direction));
    }

    /// pins a direction for a subtree
    /// helpers called inside build see it and the rest of the tree keeps its own
    pub fn scoped<R>(direction: Self, build: impl FnOnce() -> R) -> R {
        let outer = LAYOUT_DIRECTION.with(|cell| cell.replace(direction));
        let built = build();
        LAYOUT_DIRECTION.with(|cell| cell.set(outer));
        built
    }

    pub fn from_is_rtl(is_rtl: bool) -> Self {
        if is_rtl { Self::RightToLeft } else { Self::LeftToRight }
    }

    pub fn is_right_to_left(self) -> bool {
        self == Self::RightToLeft
    }
}

/// where a child sits on one physical axis, start is left or top
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AxisPlacement {
    Start,
    Center,
    End,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum HorizontalAlignment {
    Leading,
    #[default]
    Center,
    Trailing,
}

impl HorizontalAlignment {
    pub fn placement(self, direction: LayoutDirection) -> AxisPlacement {
        let rtl = direction.is_right_to_left();
        match self {
            Self::Leading if rtl => AxisPlacement::End,
            Self::Leading => AxisPlacement::Start,
            Self::Center => AxisPlacement::Center,
            Self::Trailing if rtl => AxisPlacement::Start,
            Self::Trailing => AxisPlacement::End,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum VerticalAlignment {
    Top,
    #[default]
    Center,
    Bottom,
    FirstTextBaseline,
}

impl VerticalAlignment {
    /// a frame or zstack cell holds one child so its first baseline is its top edge
    pub fn placement(self) -> AxisPlacement {
        match self {
            Self::Top | Self::FirstTextBaseline => AxisPlacement::Start,
            Self::Center => AxisPlacement::Center,
            Self::Bottom => AxisPlacement::End,
        }
    }
}

/// two axis alignment for zstack and frame
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Alignment {
    TopLeading,
    Top,
    TopTrailing,
    Leading,
    #[default]
    Center,
    Trailing,
    BottomLeading,
    Bottom,
    BottomTrailing,
}

impl Alignment {
    pub fn horizontal(self) -> HorizontalAlignment {
        match self {
            Self::TopLeading | Self::Leading | Self::BottomLeading => HorizontalAlignment::Leading,
            Self::Top | Self::Center | Self::Bottom => HorizontalAlignment::Center,
            Self::TopTrailing | Self::Trailing | Self::BottomTrailing => {
                HorizontalAlignment::Trailing
            }
        }
    }

    pub fn vertical(self) -> VerticalAlignment {
        match self {
            Self::TopLeading | Self::Top | Self::TopTrailing => VerticalAlignment::Top,
            Self::Leading | Self::Center | Self::Trailing => VerticalAlignment::Center,
            Self::BottomLeading | Self::Bottom | Self::BottomTrailing => VerticalAlignment::Bottom,
        }
    }
}

pub(crate) fn place_items<S: Styled>(element: S, placement: AxisPlacement) -> S {
    match placement {
        AxisPlacement::Start => element.items_start(),
        AxisPlacement::Center => element.items_center(),
        AxisPlacement::End => element.items_end(),
    }
}

pub(crate) fn place_justify<S: Styled>(element: S, placement: AxisPlacement) -> S {
    match placement {
        AxisPlacement::Start => element.justify_start(),
        AxisPlacement::Center => element.justify_center(),
        AxisPlacement::End => element.justify_end(),
    }
}

/// lays a single child out at the alignment inside the element, like a zstack cell or a frame box
pub(crate) fn align_single_child<S: Styled>(
    element: S,
    alignment: Alignment,
    direction: LayoutDirection,
) -> S {
    let horizontal = alignment.horizontal().placement(direction);
    let row = place_justify(element.flex().flex_row(), horizontal);
    place_items(row, alignment.vertical().placement())
}

fn spacing_px(spacing: Option<f32>) -> gpui::Pixels {
    px(spacing.unwrap_or(DEFAULT_STACK_SPACING))
}

/// vertical stack with spacing, children centered by default
/// .align changes it
pub fn vstack(spacing: impl Into<Option<f32>>) -> Div {
    div().flex().flex_col().items_center().gap(spacing_px(spacing.into()))
}

/// horizontal stack with spacing, in right to left it runs from the right edge
pub fn hstack(spacing: impl Into<Option<f32>>) -> Div {
    let row = div().flex();
    let row = if LayoutDirection::current().is_right_to_left() {
        row.flex_row_reverse()
    } else {
        row.flex_row()
    };
    row.items_center().gap(spacing_px(spacing.into()))
}

/// takes the free space on the axis of its parent stack
pub fn spacer() -> Div {
    div().flex_1()
}

/// spacer with a min length inside an hstack, parent spacing still applies on both sides
pub fn spacer_min_w(min_length: f32) -> Div {
    div().flex_1().min_w(px(min_length))
}

/// spacer with a min length inside a vstack, parent spacing still applies on both sides
pub fn spacer_min_h(min_length: f32) -> Div {
    div().flex_1().min_h(px(min_length))
}

pub trait StackAlignment: Copy {
    fn apply_to<S: Styled>(self, stack: S) -> S;
}

fn is_column(stack: &mut impl Styled) -> bool {
    matches!(
        stack.style().flex_direction,
        Some(FlexDirection::Column | FlexDirection::ColumnReverse)
    )
}

impl StackAlignment for HorizontalAlignment {
    fn apply_to<S: Styled>(self, mut stack: S) -> S {
        debug_assert!(is_column(&mut stack), "HorizontalAlignment aligns a vstack");
        place_items(stack, self.placement(LayoutDirection::current()))
    }
}

impl StackAlignment for VerticalAlignment {
    fn apply_to<S: Styled>(self, mut stack: S) -> S {
        debug_assert!(!is_column(&mut stack), "VerticalAlignment aligns an hstack");
        match self {
            Self::FirstTextBaseline => stack.items_baseline(),
            other => place_items(stack, other.placement()),
        }
    }
}

/// align method on the div that vstack and hstack return
pub trait StackAlign: Styled + Sized {
    fn align(self, alignment: impl StackAlignment) -> Self {
        alignment.apply_to(self)
    }
}

impl StackAlign for Div {}

/// one cell grid, every child shares the cell so the stack is as big as its largest child
/// later child paints and hit tests above an earlier one
#[derive(IntoElement)]
pub struct ZStack {
    alignment: Alignment,
    direction: LayoutDirection,
    base: Div,
    children: SmallVec<[AnyElement; 2]>,
}

pub fn zstack(alignment: Alignment) -> ZStack {
    ZStack {
        alignment,
        direction: LayoutDirection::current(),
        base: div(),
        children: SmallVec::new(),
    }
}

impl ParentElement for ZStack {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl Styled for ZStack {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl InteractiveElement for ZStack {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl RenderOnce for ZStack {
    fn render(mut self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let (alignment, direction) = (self.alignment, self.direction);
        let proposes_width = matches!(self.base.style().size.width, Some(Length::Definite(_)));
        let grid = if proposes_width { self.base.grid().grid_cols(1) } else { self.base.grid() };
        grid.children(self.children.into_iter().map(move |child| {
            let cell = div().row_start(1).col_start(1).size_full();
            let cell = if proposes_width { cell.min_w_0() } else { cell };
            align_single_child(cell, alignment, direction).child(child)
        }))
    }
}
