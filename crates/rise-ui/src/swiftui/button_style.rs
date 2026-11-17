use std::rc::Rc;

use gpui::{
    AnyElement, App, BoxShadow, ClickEvent, Div, ElementId, FocusHandle, Hsla, InteractiveElement, IntoElement,
    MouseButton, MouseDownEvent, ParentElement, Pixels, RenderOnce, Stateful, StatefulInteractiveElement,
    StyleRefinement, Styled, Window, div, px,
};
use rise_theme::AppTheme;

use crate::main_text::FillContentColorKey;
use crate::swiftui::environment::env_scope;
use crate::swiftui::font::{Font, SwiftUiFont};
use crate::swiftui::frame::{EdgeSet, SwiftUiFrame};
use crate::swiftui::progress_view::ControlSize;
use crate::swiftui::shapes::SwiftUiShapes;

pub type ButtonAction = Rc<dyn Fn(&mut Window, &mut App)>;

pub const HOVER_OPACITY: f32 = 0.85;

pub const DISABLED_OPACITY: f32 = 0.5;

pub const BORDERED_FILL_OPACITY: f32 = 0.15;

pub const BORDERED_PADDING_HORIZONTAL: f32 = 12.0;
pub const BORDERED_PADDING_VERTICAL: f32 = 7.0;

pub const BORDERED_CORNER_RADIUS: f32 = 8.0;

/// what the control size changes on a system bordered button
pub struct BorderedMetrics {
    pub padding_horizontal: f32,
    pub padding_vertical: f32,
    pub corner_radius: f32,
    pub font: Font,
}

/// system metrics of a bordered button per control size
/// about 24 and 28 and 34 and 50 points tall with a one line label
/// label font comes with the size unless the button sets its own
pub fn bordered_metrics(size: ControlSize) -> BorderedMetrics {
    match size {
        ControlSize::Mini => BorderedMetrics {
            padding_horizontal: 8.0,
            padding_vertical: 4.0,
            corner_radius: 6.0,
            font: Font::footnote(),
        },
        ControlSize::Small => BorderedMetrics {
            padding_horizontal: 10.0,
            padding_vertical: 5.0,
            corner_radius: 6.0,
            font: Font::subheadline(),
        },
        ControlSize::Regular => BorderedMetrics {
            padding_horizontal: BORDERED_PADDING_HORIZONTAL,
            padding_vertical: BORDERED_PADDING_VERTICAL,
            corner_radius: BORDERED_CORNER_RADIUS,
            font: Font::body(),
        },
        ControlSize::Large => BorderedMetrics {
            padding_horizontal: 20.0,
            padding_vertical: 15.0,
            corner_radius: 12.0,
            font: Font::body(),
        },
    }
}

pub const FOCUS_RING_WIDTH: f32 = 2.0;

/// focus ring for a focusable element, theme primary color drawn inside its own edge
/// only when focus came from the keyboard, never after a click
pub fn focus_visible_ring<E: InteractiveElement + Styled>(mut element: E) -> E {
    let mut shadows = element.style().box_shadow.clone().unwrap_or_default();
    shadows.push(
        BoxShadow::new(px(0.), px(0.), AppTheme::focus_ring_color())
            .spread_radius(px(FOCUS_RING_WIDTH))
            .inset(),
    );
    element.focus_visible(move |style| style.shadow(shadows))
}

/// same ring as a layer over a control with its own focus handle
/// for content that would cover an inner ring, takes no room and no hitbox
/// none unless focus came from the keyboard
pub fn keyboard_focus_ring(focus: &FocusHandle, color: Hsla, corner_radius: Pixels, window: &Window) -> Option<Div> {
    (focus.is_focused(window) && window.last_input_was_keyboard()).then(|| {
        div()
            .absolute()
            .inset_0()
            .rounded(corner_radius)
            .border_2()
            .border_color(color)
    })
}

/// makes the element a button, tab stop and hand cursor and hover tint
/// click or enter or space runs action
/// a disabled button gets none of it
pub fn button_chrome(element: Stateful<Div>, disabled: bool, action: Option<ButtonAction>) -> Stateful<Div> {
    button_chrome_hover(element, disabled, action, |style: StyleRefinement| style.opacity(HOVER_OPACITY))
}

/// button_chrome with its own pointer look instead of the hover tint
/// gpui allows one hover style per element and asserts on a second, dont chain .hover after it
pub fn button_chrome_hover(
    element: Stateful<Div>,
    disabled: bool,
    action: Option<ButtonAction>,
    hover: impl FnOnce(StyleRefinement) -> StyleRefinement,
) -> Stateful<Div> {
    if disabled {
        return element;
    }
    let element = focus_visible_ring(element.tab_index(0))
        .cursor_pointer()
        .hover(hover)
        .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, window: &mut Window, _: &mut App| {
            window.prevent_default()
        });
    match action {
        Some(action) => element.on_click(move |_: &ClickEvent, window, cx| action(window, cx)),
        None => element,
    }
}

/// plain button, label as is
/// pressed only where the label is, no fill and no press effect
pub fn plain(id: impl Into<ElementId>) -> PlainButton {
    PlainButton { id: id.into(), disabled: false, action: None, base: div() }
}

/// element from plain, styles on it style the box around the label
#[derive(IntoElement)]
pub struct PlainButton {
    id: ElementId,
    disabled: bool,
    action: Option<ButtonAction>,
    base: Div,
}

impl PlainButton {
    pub fn on_press(mut self, action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.action = Some(Rc::new(action));
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl Styled for PlainButton {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl ParentElement for PlainButton {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.base.extend(elements);
    }
}

impl RenderOnce for PlainButton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        button_chrome(self.base.id(self.id), self.disabled, self.action)
    }
}

/// bordered or bordered prominent system button style
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BorderedStyle {
    Bordered,
    Prominent,
}

pub fn system_button(id: impl Into<ElementId>, style: BorderedStyle) -> SystemButton {
    SystemButton {
        id: id.into(),
        style,
        tint: None,
        control_size: ControlSize::Regular,
        disabled: false,
        action: None,
        children: Vec::new(),
    }
}

#[derive(IntoElement)]
pub struct SystemButton {
    id: ElementId,
    style: BorderedStyle,
    tint: Option<Hsla>,
    control_size: ControlSize,
    disabled: bool,
    action: Option<ButtonAction>,
    children: Vec<AnyElement>,
}

impl SystemButton {
    pub fn on_press(mut self, action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.action = Some(Rc::new(action));
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// tint color, accent color when not set
    pub fn tint(mut self, color: Hsla) -> Self {
        self.tint = Some(color);
        self
    }

    pub fn control_size(mut self, size: ControlSize) -> Self {
        self.control_size = size;
        self
    }
}

impl ParentElement for SystemButton {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

/// fill and label ink of a bordered button with this tint
pub fn bordered_colors(style: BorderedStyle, tint: Hsla, content_on_tint: Hsla) -> (Hsla, Hsla) {
    match style {
        BorderedStyle::Bordered => (tint.opacity(BORDERED_FILL_OPACITY), tint),
        BorderedStyle::Prominent => (tint, content_on_tint),
    }
}

impl RenderOnce for SystemButton {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = crate::theme(cx);
        let tint = self.tint.unwrap_or(theme.primary._100);
        let (fill, label) = bordered_colors(self.style, tint, theme.content_on(tint));
        let metrics = bordered_metrics(self.control_size);

        let surface = div()
            .flex()
            .items_center()
            .justify_center()
            .padding_edges(EdgeSet::HORIZONTAL, metrics.padding_horizontal)
            .padding_edges(EdgeSet::VERTICAL, metrics.padding_vertical)
            .swiftui_font(metrics.font)
            .bg(fill)
            .text_color(label)
            .continuous_corners(metrics.corner_radius)
            .opacity(if self.disabled { DISABLED_OPACITY } else { 1.0 })
            .child(env_scope::<FillContentColorKey>(Some(label)).children(self.children));

        button_chrome(surface.id(self.id), self.disabled, self.action)
    }
}
