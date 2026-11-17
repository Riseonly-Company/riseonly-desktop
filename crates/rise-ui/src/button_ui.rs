use std::rc::Rc;

use gpui::{
    AnyElement, App, ElementId, Hsla, IntoElement, ParentElement, Pixels, RenderOnce, Styled,
    Window, div, px,
};
use smallvec::SmallVec;

use crate::main_text::FillContentColorKey;
use crate::swiftui::button_style::{ButtonAction, DISABLED_OPACITY, plain};
use crate::swiftui::environment::env_scope;
use crate::swiftui::frame::{EdgeSet, SwiftUiFrame};
use crate::swiftui::progress_view::progress_view;
use crate::swiftui::shapes::{Shape, SwiftUiShapes};
use crate::swiftui::stacks::hstack;

/// colors and height a surface resolves from the theme and its parameters
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ButtonUiSurfaceColors {
    pub background: Hsla,
    pub height: Pixels,
    pub fill_content: Hsla,
    pub loader: Hsla,
}

/// button chrome without the button, for places where the press belongs to a system container
/// eg a menu, a nested button in its label would swallow the click
#[derive(IntoElement)]
pub struct ButtonUiSurface {
    disabled: bool,
    is_pending: bool,
    background_color: Option<Hsla>,
    height: Option<f32>,
    fit_content: bool,
    padding_horizontal: f32,
    padding_vertical: f32,
    gap: f32,
    loader_color: Option<Hsla>,
    children: SmallVec<[AnyElement; 2]>,
}

impl Default for ButtonUiSurface {
    fn default() -> Self {
        Self::new()
    }
}

impl ButtonUiSurface {
    pub fn new() -> Self {
        Self {
            disabled: false,
            is_pending: false,
            background_color: None,
            height: None,
            fit_content: false,
            padding_horizontal: 0.0,
            padding_vertical: 0.0,
            gap: 0.0,
            loader_color: None,
            children: SmallVec::new(),
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn is_pending(mut self, is_pending: bool) -> Self {
        self.is_pending = is_pending;
        self
    }

    pub fn background_color(mut self, color: Hsla) -> Self {
        self.background_color = Some(color);
        self
    }

    pub fn height(mut self, height: f32) -> Self {
        self.height = Some(height);
        self
    }

    pub fn fit_content(mut self, fit_content: bool) -> Self {
        self.fit_content = fit_content;
        self
    }

    pub fn padding_horizontal(mut self, padding: f32) -> Self {
        self.padding_horizontal = padding;
        self
    }

    pub fn padding_vertical(mut self, padding: f32) -> Self {
        self.padding_vertical = padding;
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    pub fn loader_color(mut self, color: Hsla) -> Self {
        self.loader_color = Some(color);
        self
    }

    /// background, height and inks for this surface on theme
    pub fn resolved_colors(&self, theme: &rise_theme::AppTheme) -> ButtonUiSurfaceColors {
        let background = self.background_color.unwrap_or(theme.primary._100);
        let fill_content = theme.content_on(background);
        ButtonUiSurfaceColors {
            background,
            height: self.height.map(px).unwrap_or(theme.button.height_300),
            fill_content,
            loader: self.loader_color.unwrap_or(fill_content),
        }
    }
}

impl ParentElement for ButtonUiSurface {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for ButtonUiSurface {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = self.resolved_colors(crate::theme(cx));

        let row = hstack(self.gap)
            .padding_edges(EdgeSet::HORIZONTAL, self.padding_horizontal)
            .padding_edges(EdgeSet::VERTICAL, self.padding_vertical);

        let surface = div()
            .flex()
            .items_center()
            .justify_center()
            .h(colors.height)
            .bg(colors.background)
            .clip_shape(Shape::Capsule)
            .opacity(if self.disabled { DISABLED_OPACITY } else { 1.0 });
        let surface = if self.fit_content { surface } else { surface.w_full() };

        if self.is_pending {
            surface.child(row.child(progress_view().tint(colors.loader)))
        } else {
            surface.child(env_scope::<FillContentColorKey>(Some(colors.fill_content)).child(row.children(self.children)))
        }
    }
}

/// the filled capsule button, a ButtonUiSurface inside a plain button
/// a disabled or pending button doesnt press
#[derive(IntoElement)]
pub struct ButtonUi {
    id: ElementId,
    surface: ButtonUiSurface,
    on_press: Option<ButtonAction>,
}

impl ButtonUi {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self { id: id.into(), surface: ButtonUiSurface::new(), on_press: None }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.surface = self.surface.disabled(disabled);
        self
    }

    pub fn is_pending(mut self, is_pending: bool) -> Self {
        self.surface = self.surface.is_pending(is_pending);
        self
    }

    pub fn background_color(mut self, color: Hsla) -> Self {
        self.surface = self.surface.background_color(color);
        self
    }

    pub fn height(mut self, height: f32) -> Self {
        self.surface = self.surface.height(height);
        self
    }

    pub fn fit_content(mut self, fit_content: bool) -> Self {
        self.surface = self.surface.fit_content(fit_content);
        self
    }

    pub fn padding_horizontal(mut self, padding: f32) -> Self {
        self.surface = self.surface.padding_horizontal(padding);
        self
    }

    pub fn padding_vertical(mut self, padding: f32) -> Self {
        self.surface = self.surface.padding_vertical(padding);
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.surface = self.surface.gap(gap);
        self
    }

    pub fn loader_color(mut self, color: Hsla) -> Self {
        self.surface = self.surface.loader_color(color);
        self
    }

    pub fn on_press(mut self, action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_press = Some(Rc::new(action));
        self
    }

    /// whether a press runs the action, disabled or pending blocks it
    pub fn accepts_press(&self) -> bool {
        !self.surface.disabled && !self.surface.is_pending
    }
}

impl ParentElement for ButtonUi {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.surface.extend(elements);
    }
}

impl RenderOnce for ButtonUi {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let accepts_press = self.accepts_press();
        let fit_content = self.surface.fit_content;
        let mut button = plain(self.id).disabled(!accepts_press);
        if let Some(on_press) = self.on_press {
            button = button.on_press(move |window, cx| {
                if accepts_press {
                    on_press(window, cx);
                }
            });
        }
        let button = if fit_content { button } else { button.w_full() };
        button.child(self.surface)
    }
}
