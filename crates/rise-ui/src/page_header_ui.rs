use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AnyElement, App, AvailableSpace, Bounds, DefiniteLength, Edges, Element, ElementId, FontWeight,
    GlobalElementId, InspectorElementId, InteractiveElement, IntoElement, LayoutId, Length, ParentElement, Pixels,
    Point, Position, RenderOnce, SharedString, Size, Style, Styled, Window, div, point, px, relative,
};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::i18n::tr;
use smallvec::SmallVec;

use crate::liquid_chrome::LiquidChromeModifiers;
use crate::main_text::{IgnoresTextSizeDeltaKey, MainText, TextAlignT};
use crate::page_header_toolbar_button::{
    PageHeaderAction, PageHeaderBackButton, PageHeaderIconButton, PageHeaderMetrics,
};
use crate::swiftui::environment::{EnvKey, env, env_scope, layout_direction};
use crate::swiftui::frame::SwiftUiAlignedFrame;
use crate::swiftui::stacks::{Alignment, hstack, vstack};
use crate::swiftui::tooltip_label::TooltipLabelModifiers;

const BACK_BUTTON_ID: &str = "page-header-back";
const PANEL_BACK_ID: &str = "panel-chrome-back";
const PANEL_OPEN_IN_CENTRE_ID: &str = "panel-chrome-open-in-centre";
const PANEL_CLOSE_ID: &str = "panel-chrome-close";
const PANEL_BACK_KEY: &str = "back";
const PANEL_OPEN_IN_CENTRE_KEY: &str = "aside_open_in_centre";
const PANEL_CLOSE_KEY: &str = "desktop_panel_close";
const PANEL_OPEN_IN_CENTRE_SYMBOL: &str = "arrow.up.left.and.arrow.down.right";
const PANEL_CLOSE_SYMBOL: &str = "xmark";

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

/// back, open in centre and close a box of the window hands to the page inside it
#[derive(Clone)]
pub struct PanelChrome {
    back: Option<PageHeaderAction>,
    open_in_centre: Option<PageHeaderAction>,
    close: Option<PageHeaderAction>,
    taken: Rc<Cell<bool>>,
}

#[derive(Clone)]
pub struct PanelChromeMenuAction {
    pub id: &'static str,
    pub label: SharedString,
    pub system_image: &'static str,
    pub action: PageHeaderAction,
}

impl PanelChrome {
    pub fn new(taken: Rc<Cell<bool>>) -> Self {
        Self { back: None, open_in_centre: None, close: None, taken }
    }

    pub fn back(mut self, back: Option<PageHeaderAction>) -> Self {
        self.back = back;
        self
    }

    pub fn open_in_centre(mut self, open_in_centre: Option<PageHeaderAction>) -> Self {
        self.open_in_centre = open_in_centre;
        self
    }

    pub fn close(mut self, close: Option<PageHeaderAction>) -> Self {
        self.close = close;
        self
    }

    pub fn with_taken(mut self, taken: Rc<Cell<bool>>) -> Self {
        self.taken = taken;
        self
    }

    pub fn close_action(&self) -> Option<PageHeaderAction> {
        self.close.clone()
    }

    pub fn current(cx: &App) -> Option<Self> {
        env::<PanelChromeKey>(cx)
    }

    /// chrome of the box around marked as placed, the box then keeps no row of its own
    pub fn take(cx: &App) -> Option<Self> {
        let chrome = env::<PanelChromeKey>(cx)?;
        chrome.taken.set(true);
        Some(chrome)
    }

    fn button(id: &'static str, symbol: &'static str, key: &'static str, action: PageHeaderAction) -> AnyElement {
        div()
            .id(id)
            .tooltip_label(tr(key))
            .child(PageHeaderIconButton::new(id, symbol, move |window, cx| action(window, cx)))
            .into_any_element()
    }

    pub fn leading_button(&self) -> Option<AnyElement> {
        let back = self.back.clone()?;
        Some(
            div()
                .id(PANEL_BACK_ID)
                .tooltip_label(tr(PANEL_BACK_KEY))
                .child(PageHeaderBackButton::from_action(PANEL_BACK_ID, back))
                .into_any_element(),
        )
    }

    /// open in centre then close, none when the box offers neither
    pub fn trailing_buttons(&self) -> Option<AnyElement> {
        if self.open_in_centre.is_none() && self.close.is_none() {
            return None;
        }
        let open = self
            .open_in_centre
            .clone()
            .map(|action| Self::button(PANEL_OPEN_IN_CENTRE_ID, PANEL_OPEN_IN_CENTRE_SYMBOL, PANEL_OPEN_IN_CENTRE_KEY, action));
        let close = self.close.clone().map(|action| Self::button(PANEL_CLOSE_ID, PANEL_CLOSE_SYMBOL, PANEL_CLOSE_KEY, action));
        Some(hstack(PageHeaderMetrics::BUTTON_GAP).items_center().children(open).children(close).into_any_element())
    }

    /// row the box draws over a page that has no header of its own
    pub fn row(&self) -> AnyElement {
        hstack(PageHeaderMetrics::BUTTON_GAP)
            .w_full()
            .h(px(PageHeaderMetrics::PANEL_STRIP_HEIGHT))
            .pt(px(PageHeaderMetrics::TOP_INSET))
            .px(px(PageHeaderMetrics::BAR_EDGE_INSET))
            .items_center()
            .children(self.leading_button())
            .child(div().flex_1())
            .children(self.trailing_buttons())
            .into_any_element()
    }

    pub fn menu_actions(&self) -> Vec<PanelChromeMenuAction> {
        let open = self.open_in_centre.clone().map(|action| PanelChromeMenuAction {
            id: PANEL_OPEN_IN_CENTRE_ID,
            label: tr(PANEL_OPEN_IN_CENTRE_KEY).into(),
            system_image: PANEL_OPEN_IN_CENTRE_SYMBOL,
            action,
        });
        let close = self.close.clone().map(|action| PanelChromeMenuAction {
            id: PANEL_CLOSE_ID,
            label: tr(PANEL_CLOSE_KEY).into(),
            system_image: PANEL_CLOSE_SYMBOL,
            action,
        });
        open.into_iter().chain(close).collect()
    }

    pub fn run_menu_action(&self, id: &str, window: &mut Window, cx: &mut App) -> bool {
        let action = match id {
            PANEL_OPEN_IN_CENTRE_ID => self.open_in_centre.clone(),
            PANEL_CLOSE_ID => self.close.clone(),
            _ => None,
        };
        let Some(action) = action else {
            return false;
        };
        action(window, cx);
        true
    }
}

/// set by a box of the window around the page it shows, none elsewhere
pub struct PanelChromeKey;

impl EnvKey for PanelChromeKey {
    type Value = Option<PanelChrome>;

    fn default_value() -> Self::Value {
        None
    }

    fn fingerprint(value: &Self::Value) -> Option<u64> {
        value.as_ref().map(|chrome| {
            8 | u64::from(chrome.back.is_some())
                | u64::from(chrome.open_in_centre.is_some()) << 1
                | u64::from(chrome.close.is_some()) << 2
        })
    }
}

/// lays the page out first and then gives the box row only to a page that left the chrome untaken
pub struct PanelChromeSlot {
    taken: Rc<Cell<bool>>,
    row_height: f32,
    page: AnyElement,
    row: Option<AnyElement>,
}

impl PanelChromeSlot {
    pub fn new(taken: Rc<Cell<bool>>, page: impl IntoElement, row: Option<AnyElement>) -> Self {
        Self { taken, row_height: PageHeaderMetrics::PANEL_STRIP_HEIGHT, page: page.into_any_element(), row }
    }

    fn pinned(top: f32, height: Option<f32>) -> Style {
        let mut style = Style::default();
        style.position = Position::Absolute;
        style.inset = Edges {
            top: px(top).into(),
            right: px(0.0).into(),
            bottom: if height.is_some() { Length::Auto } else { px(0.0).into() },
            left: px(0.0).into(),
        };
        if let Some(height) = height {
            style.size.height = Length::Definite(DefiniteLength::from(px(height)));
        }
        style
    }
}

impl IntoElement for PanelChromeSlot {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for PanelChromeSlot {
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
    ) -> (LayoutId, ()) {
        let page = self.page.request_layout(window, cx);
        if self.taken.get() {
            self.row = None;
        }
        let page_top = if self.row.is_some() { self.row_height } else { 0.0 };
        let mut children = vec![window.request_layout(Self::pinned(page_top, None), [page], cx)];
        if let Some(row) = self.row.as_mut() {
            let row = row.request_layout(window, cx);
            children.push(window.request_layout(Self::pinned(0.0, Some(self.row_height)), [row], cx));
        }
        (window.request_layout(Self::pinned(0.0, None), children, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.page.prepaint(window, cx);
        if self.row.is_some() && self.taken.get() {
            self.row = None;
            window.request_animation_frame();
            return;
        }
        if let Some(row) = self.row.as_mut() {
            row.prepaint(window, cx);
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        _prepaint: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.page.paint(window, cx);
        if let Some(row) = self.row.as_mut() {
            row.paint(window, cx);
        }
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
    panel_chrome_in_menu: bool,
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
            panel_chrome_in_menu: false,
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

    pub fn panel_chrome_in_menu(mut self, panel_chrome_in_menu: bool) -> Self {
        self.panel_chrome_in_menu = panel_chrome_in_menu;
        self
    }

    fn take_panel_chrome(&mut self, cx: &App) {
        let Some(chrome) = PanelChrome::take(cx) else {
            return;
        };
        if let Some(back) = chrome.back.clone() {
            self.shows_back_button = true;
            self.on_back = back;
        }
        if self.panel_chrome_in_menu {
            return;
        }
        let Some(trailing) = chrome.trailing_buttons() else {
            return;
        };
        self.right_jsx = Some(match self.right_jsx.take() {
            Some(right) => hstack(PageHeaderMetrics::BUTTON_GAP).items_center().child(right).child(trailing).into_any_element(),
            None => trailing,
        });
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
        let mut row = hstack(PageHeaderMetrics::BUTTON_GAP)
            .px(px(PageHeaderMetrics::EDGE_INSET))
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
        self.take_panel_chrome(cx);
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
            edge_padding: PageHeaderMetrics::BAR_EDGE_INSET,
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
