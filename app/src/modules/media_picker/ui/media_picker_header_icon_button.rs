use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, ElementId, IntoElement, RenderOnce, SharedString, Window, white};
use rise_ui::liquid_chrome::{LiquidChrome, gradient_stroke_color};
use rise_ui::page_header_toolbar_button::PageHeaderIconButton;
use rise_ui::swiftui::prelude::*;

const HOVER_HIGHLIGHT_OPACITY: f32 = 0.06;

#[derive(IntoElement)]
pub struct MediaPickerHeaderIconButton {
    pub id: ElementId,
    pub system_image: SharedString,
    pub font_size: f32,
    pub disabled: bool,
    pub action: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl MediaPickerHeaderIconButton {
    /// defaults are font_size 17 and disabled false
    pub fn new(
        id: impl Into<ElementId>,
        system_image: impl Into<SharedString>,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            system_image: system_image.into(),
            font_size: 17.0,
            disabled: false,
            action: Rc::new(action),
        }
    }

    pub fn font_size(mut self, font_size: f32) -> Self {
        self.font_size = font_size;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl RenderOnce for MediaPickerHeaderIconButton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let action = self.action.clone();
        let button = PageHeaderIconButton::new(self.id.clone(), self.system_image.clone(), move |window, cx| {
            action(window, cx)
        })
        .font_size(self.font_size)
        .disabled(self.disabled);

        let stroke = centred_stroke(
            gradient_stroke_color(0.20, 0.06),
            LiquidChrome::nav_glass_border_width(),
            Shape::Circle,
        );

        let disabled = self.disabled;
        let hover = div()
            .id(ElementId::NamedChild(Arc::new(self.id), "hover".into()))
            .absolute()
            .inset_0()
            .rounded_full()
            .when(!disabled, |hover| {
                hover.hover(|style| style.bg(white().opacity(HOVER_HIGHLIGHT_OPACITY)))
            });

        div().relative().child(button).child(hover).child(stroke)
    }
}
