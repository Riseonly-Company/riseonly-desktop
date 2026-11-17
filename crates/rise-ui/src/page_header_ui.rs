use std::rc::Rc;

use gpui::{
    AnyElement, App, AvailableSpace, Bounds, DefiniteLength, Element, ElementId, FontWeight,
    GlobalElementId, InspectorElementId, IntoElement, LayoutId, Length, ParentElement, Pixels,
    Point, RenderOnce, SharedString, Size, Style, Styled, Window, div, point, px, relative,
};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use smallvec::SmallVec;

use crate::liquid_chrome::LiquidChromeModifiers;
use crate::main_text::{IgnoresTextSizeDeltaKey, MainText, TextAlignT};
use crate::page_header_toolbar_button::{PageHeaderAction, PageHeaderBackButton, PageHeaderMetrics};
use crate::swiftui::environment::{EnvKey, env, env_scope, layout_direction};
use crate::swiftui::frame::SwiftUiAlignedFrame;
use crate::swiftui::stacks::{Alignment, hstack, vstack};

const BACK_BUTTON_ID: &str = "page-header-back";

/// the page is the first one beside a split list (the settings sections)
/// the list stays on screen so there is nothing to go back to and no back is drawn
pub struct PageBesideSplitListKey;

impl EnvKey for PageBesideSplitListKey {
    type Value = bool;

    fn default_value() -> Self::Value {
        false
    }
}

/// page lies under one sliding in over it or is itself leaving on a pop, still drawn but soon gone
/// its own motion settles at once instead of rendering the page every frame
pub struct PageCoveredByArrivingPageKey;

impl EnvKey for PageCoveredByArrivingPageKey {
    type Value = bool;

    fn default_value() -> Self::Value {
        false
    }

    fn fingerprint(_value: &Self::Value) -> Option<u64> {
        None
    }
}

/// room the window panel toggle takes at the trailing end of a header row when the page runs
/// under it (eg a profile cover edge to edge)
/// zero elsewhere
pub struct PanelToggleHeaderReserveKey;

impl EnvKey for PanelToggleHeaderReserveKey {
    type Value = gpui::Pixels;

    fn default_value() -> Self::Value {
        gpui::px(0.0)
    }

    fn fingerprint(_value: &Self::Value) -> Option<u64> {
        None
    }
}

/// the bar a page opens with, back or leading control
/// trailing control and a title or center slot
#[derive(IntoElement)]
pub struct PageHeaderUi {
    title: Option<SharedString>,
    subtitle: Option<SharedString>,
    shows_back_button: bool,
    on_back: PageHeaderAction,
    right_jsx: Option<AnyElement>,
    left_jsx: Option<AnyElement>,
    center_slot: Option<AnyElement>,
    center_slot_self_styled: bool,
    center_slot_fills_width: bool,
}

impl Default for PageHeaderUi {
    fn default() -> Self {
        Self {
            title: None,
            subtitle: None,
            shows_back_button: true,
            on_back: Rc::new(|_, _| {}),
            right_jsx: None,
            left_jsx: None,
            center_slot: None,
            center_slot_self_styled: false,
            center_slot_fills_width: false,
        }
    }
}

impl PageHeaderUi {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn title(mut self, title: impl Into<Option<SharedString>>) -> Self {
        self.title = title.into();
        self
    }

    pub fn subtitle(mut self, subtitle: impl Into<Option<SharedString>>) -> Self {
        self.subtitle = subtitle.into();
        self
    }

    pub fn shows_back_button(mut self, shows_back_button: bool) -> Self {
        self.shows_back_button = shows_back_button;
        self
    }

    pub fn on_back(mut self, on_back: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_back = Rc::new(on_back);
        self
    }

    pub fn right_jsx(mut self, right_jsx: impl IntoElement) -> Self {
        self.right_jsx = Some(right_jsx.into_any_element());
        self
    }

    pub fn left_jsx(mut self, left_jsx: impl IntoElement) -> Self {
        self.left_jsx = Some(left_jsx.into_any_element());
        self
    }

    pub fn center_slot(mut self, center_slot: impl IntoElement) -> Self {
        self.center_slot = Some(center_slot.into_any_element());
        self
    }

    pub fn center_slot_self_styled(mut self, center_slot_self_styled: bool) -> Self {
        self.center_slot_self_styled = center_slot_self_styled;
        self
    }

    pub fn center_slot_fills_width(mut self, center_slot_fills_width: bool) -> Self {
        self.center_slot_fills_width = center_slot_fills_width;
        self
    }

    pub fn trimmed_title(&self) -> Option<SharedString> {
        Self::trimmed(self.title.as_ref())
    }

    pub fn trimmed_subtitle(&self) -> Option<SharedString> {
        Self::trimmed(self.subtitle.as_ref())
    }

    pub fn shows_default_text_center(&self) -> bool {
        self.trimmed_title().is_some() || self.trimmed_subtitle().is_some()
    }

    fn trimmed(text: Option<&SharedString>) -> Option<SharedString> {
        let trimmed = trimming_whitespaces_and_newlines(text?.as_ref());
        (!trimmed.is_empty()).then(|| SharedString::from(trimmed.to_owned()))
    }

    fn side_placeholder() -> AnyElement {
        let side = px(PageHeaderMetrics::TOOLBAR_BUTTON_SIZE);
        div().w(side).h(side).into_any_element()
    }

    fn filled_bar(self, center: AnyElement) -> AnyElement {
        let mut row = hstack(10.0)
            .px(px(12.0))
            .h(px(PageHeaderMetrics::BAR_HEIGHT));
        if self.shows_back_button {
            row = row.child(PageHeaderBackButton::from_action(BACK_BUTTON_ID, self.on_back));
        } else if let Some(left) = self.left_jsx {
            row = row.child(left);
        }
        row = row.child(center.frame_max_width(Alignment::Center).flex_1());
        if let Some(right) = self.right_jsx {
            row = row.child(right);
        }
        row.into_any_element()
    }

    fn overlay_bar(self) -> AnyElement {
        let default_text_center = self.default_text_center();
        let leading = if self.shows_back_button {
            PageHeaderBackButton::from_action(BACK_BUTTON_ID, self.on_back).into_any_element()
        } else if let Some(left) = self.left_jsx {
            left
        } else {
            Self::side_placeholder()
        };
        let trailing = self.right_jsx.unwrap_or_else(Self::side_placeholder);

        let mut bar = PageHeaderBarLayout::new()
            .height(PageHeaderMetrics::BAR_HEIGHT)
            .child(leading)
            .child(trailing);
        if let Some(center) = self.center_slot {
            bar = if self.center_slot_self_styled {
                bar.child(center)
            } else {
                bar.child(center.liquid_glass_nav_pill(10.0, None))
            };
        } else if let Some(text) = default_text_center {
            bar = bar.child(text);
        }
        bar.into_any_element()
    }

    fn default_text_center(&self) -> Option<AnyElement> {
        if !self.shows_default_text_center() {
            return None;
        }
        let mut column = vstack(0.0);
        if let Some(subtitle) = self.trimmed_subtitle() {
            column = column.child(
                MainText::new(subtitle)
                    .px(11.0)
                    .tac(TextAlignT::Center)
                    .number_of_lines(1),
            );
        }
        if let Some(title) = self.trimmed_title() {
            column = column.child(
                MainText::new(title)
                    .px(15.0)
                    .tac(TextAlignT::Center)
                    .font_weight(FontWeight::BOLD)
                    .number_of_lines(1),
            );
        }
        Some(column.into_any_element())
    }
}

impl RenderOnce for PageHeaderUi {
    fn render(mut self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        if env::<PageBesideSplitListKey>(cx) {
            self.shows_back_button = false;
        }
        let bar = if self.center_slot_fills_width
            && let Some(center) = self.center_slot.take()
        {
            self.filled_bar(center)
        } else {
            self.overlay_bar()
        };
        env_scope::<IgnoresTextSizeDeltaKey>(true)
            .child(div().w_full().pt(px(PageHeaderMetrics::TOP_INSET)).child(bar))
    }
}

/// where the center lands inside the bar, in the bar left to right coordinates
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageHeaderCenterPlacement {
    pub mid_x: f32,
    pub width: f32,
}

/// leading and trailing controls sit at the bar edges, center sits in the middle at its ideal width
/// a center that would run under a wider side slides toward the narrower one
/// only past the room between the controls is it truncated
pub struct PageHeaderBarLayout {
    edge_padding: f32,
    center_gap: f32,
    height: Option<f32>,
    children: SmallVec<[AnyElement; 3]>,
}

impl Default for PageHeaderBarLayout {
    fn default() -> Self {
        Self {
            edge_padding: 8.0,
            center_gap: 4.0,
            height: None,
            children: SmallVec::new(),
        }
    }
}

impl PageHeaderBarLayout {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn edge_padding(mut self, edge_padding: f32) -> Self {
        self.edge_padding = edge_padding;
        self
    }

    pub fn center_gap(mut self, center_gap: f32) -> Self {
        self.center_gap = center_gap;
        self
    }

    /// fixed height of the layout, without it the bar is as tall as its tallest child
    pub fn height(mut self, height: f32) -> Self {
        self.height = Some(height);
        self
    }

    /// center slot given the bar horizontal extent, both side widths and the center ideal width
    pub fn center_placement(
        &self,
        min_x: f32,
        max_x: f32,
        leading_width: f32,
        trailing_width: f32,
        ideal_width: f32,
    ) -> PageHeaderCenterPlacement {
        let low = min_x + self.edge_padding + leading_width + self.center_gap;
        let high = max_x - self.edge_padding - trailing_width - self.center_gap;
        let width = ideal_width.min((high - low).max(0.0));
        let mid = (min_x + max_x) / 2.0;
        let mid_x = mid.max(low + width / 2.0).min(high - width / 2.0);
        PageHeaderCenterPlacement { mid_x, width }
    }

    fn ideal_space() -> Size<AvailableSpace> {
        Size {
            width: AvailableSpace::MaxContent,
            height: AvailableSpace::MaxContent,
        }
    }
}

impl ParentElement for PageHeaderBarLayout {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl IntoElement for PageHeaderBarLayout {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

const CENTER_TRUNCATION_SLACK: f32 = 8.0;

fn physical_left(bounds: &Bounds<Pixels>, left: Pixels, width: Pixels, rtl: bool) -> Pixels {
    if rtl {
        bounds.left() + (bounds.right() - (left + width))
    } else {
        left
    }
}

fn centered_top(bounds: &Bounds<Pixels>, height: Pixels) -> Pixels {
    bounds.center().y - height * 0.5
}

impl Element for PageHeaderBarLayout {
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
        let height = match self.height {
            Some(height) => height,
            None => {
                let mut tallest = px(0.0);
                for child in &mut self.children {
                    let size = child.layout_as_root(Self::ideal_space(), window, cx);
                    tallest = tallest.max(size.height);
                }
                f32::from(tallest)
            }
        };
        let mut style = Style::default();
        style.size.width = Length::Definite(relative(1.0));
        style.size.height = Length::Definite(DefiniteLength::from(px(height)));
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        if self.children.len() < 2 {
            return;
        }
        let rtl = layout_direction(cx).is_right_to_left();
        let edge = px(self.edge_padding);

        let leading = self.children[0].layout_as_root(Self::ideal_space(), window, cx);
        let trailing = self.children[1].layout_as_root(Self::ideal_space(), window, cx);

        let leading_left = physical_left(&bounds, bounds.left() + edge, leading.width, rtl);
        self.children[0].prepaint_at(
            point(leading_left, centered_top(&bounds, leading.height)),
            window,
            cx,
        );
        let trailing_left = physical_left(
            &bounds,
            bounds.right() - edge - trailing.width,
            trailing.width,
            rtl,
        );
        self.children[1].prepaint_at(
            point(trailing_left, centered_top(&bounds, trailing.height)),
            window,
            cx,
        );

        if self.children.len() < 3 {
            return;
        }
        let ideal = self.children[2].layout_as_root(Self::ideal_space(), window, cx);
        let placement = self.center_placement(
            f32::from(bounds.left()),
            f32::from(bounds.right()),
            f32::from(leading.width),
            f32::from(trailing.width),
            f32::from(ideal.width) + CENTER_TRUNCATION_SLACK,
        );
        let offered = Size {
            width: AvailableSpace::Definite(px(placement.width)),
            height: AvailableSpace::Definite(ideal.height),
        };
        let placed = self.children[2].layout_as_root(offered, window, cx);
        let left = px(placement.mid_x) - placed.width * 0.5;
        let origin: Point<Pixels> = point(
            physical_left(&bounds, left, placed.width, rtl),
            centered_top(&bounds, placed.height),
        );
        self.children[2].prepaint_at(origin, window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if self.children.len() < 2 {
            return;
        }
        for child in &mut self.children {
            child.paint(window, cx);
        }
    }
}

#[cfg(test)]
#[path = "page_header_ui_tests.rs"]
mod tests;
