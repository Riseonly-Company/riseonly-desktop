use std::rc::Rc;

use gpui::{App, ElementId, FontWeight, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div, px};
use rise_canon::RiseGuideAnchor;

use crate::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use crate::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use crate::main_text::MainText;
use crate::page_header_toolbar_button::{PageHeaderAction, PageHeaderIconButton};
use crate::swiftui::shapes::pass_through;
use crate::swiftui::stacks::{Alignment, hstack, spacer, spacer_min_w, zstack};

const SIDE_MIN_LENGTH: f32 = 56.0;
const DISABLED_OPACITY: f32 = 0.35;

/// bar on top of a sheet, centered title between a leading and an optional trailing glyph
#[derive(IntoElement)]
pub struct SheetHeaderBar {
    id: ElementId,
    title: SharedString,
    left_system_image: SharedString,
    right_system_image: Option<SharedString>,
    right_text: Option<SharedString>,
    right_enabled: bool,
    on_left: PageHeaderAction,
    on_right: PageHeaderAction,
    left_guide_anchor: Option<RiseGuideAnchor>,
    right_guide_anchor: Option<RiseGuideAnchor>,
}

impl SheetHeaderBar {
    /// defaults are leading xmark and trailing checkmark enabled, on_right does nothing
    pub fn new(
        id: impl Into<ElementId>,
        title: impl Into<SharedString>,
        on_left: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            left_system_image: "xmark".into(),
            right_system_image: Some("checkmark".into()),
            right_text: None,
            right_enabled: true,
            on_left: Rc::new(on_left),
            on_right: Rc::new(|_, _| {}),
            left_guide_anchor: None,
            right_guide_anchor: None,
        }
    }

    pub fn left_system_image(mut self, system_image: impl Into<SharedString>) -> Self {
        self.left_system_image = system_image.into();
        self
    }

    pub fn right_system_image(mut self, system_image: Option<SharedString>) -> Self {
        self.right_system_image = system_image;
        self
    }

    /// text trailing button (eg save) in place of the glyph
    pub fn right_text(mut self, text: impl Into<SharedString>) -> Self {
        self.right_text = Some(text.into());
        self
    }

    pub fn right_enabled(mut self, enabled: bool) -> Self {
        self.right_enabled = enabled;
        self
    }

    pub fn on_right(mut self, on_right: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_right = Rc::new(on_right);
        self
    }

    pub fn left_guide_anchor(mut self, anchor: Option<RiseGuideAnchor>) -> Self {
        self.left_guide_anchor = anchor;
        self
    }

    pub fn right_guide_anchor(mut self, anchor: Option<RiseGuideAnchor>) -> Self {
        self.right_guide_anchor = anchor;
        self
    }
}

impl RenderOnce for SheetHeaderBar {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let text_primary = crate::theme(cx).text.primary;
        if self.right_text.is_some() {
            return self.render_text_trailing(text_primary).into_any_element();
        }
        let side = SIDE_MIN_LENGTH;
        let title = pass_through().w_full().child(
            hstack(0.0)
                .w_full()
                .child(spacer_min_w(side))
                .child(
                    MainText::new(self.title)
                        .px(17.0)
                        .font_weight(FontWeight::SEMIBOLD)
                        .color(text_primary)
                        .number_of_lines(1),
                )
                .child(spacer_min_w(side)),
        );

        let on_left = self.on_left.clone();
        let left = PageHeaderIconButton::new(
            ElementId::NamedChild(std::sync::Arc::new(self.id.clone()), "left".into()),
            self.left_system_image,
            move |window, cx| on_left(window, cx),
        )
        .font_size(15.0)
        .guide_anchor(self.left_guide_anchor);

        let mut buttons = hstack(None).w_full().child(left).child(spacer());
        if let Some(right_system_image) = self.right_system_image {
            let right_enabled = self.right_enabled;
            let on_right = self.on_right.clone();
            let right = PageHeaderIconButton::new(
                ElementId::NamedChild(std::sync::Arc::new(self.id.clone()), "right".into()),
                right_system_image,
                move |window, cx| {
                    if right_enabled {
                        on_right(window, cx);
                    }
                },
            )
            .font_size(16.0)
            .disabled(!right_enabled);
            buttons = buttons.child(
                div()
                    .opacity(if right_enabled { 1.0 } else { DISABLED_OPACITY })
                    .child(right)
                    .guide_anchor(self.right_guide_anchor),
            );
        }

        zstack(Alignment::Center)
            .w_full()
            .child(title)
            .child(buttons)
            .px(px(14.0))
            .py(px(10.0))
            .into_any_element()
    }
}

impl SheetHeaderBar {
    fn render_text_trailing(self, text_primary: gpui::Hsla) -> gpui::Div {
        let on_left = self.on_left.clone();
        let left = PageHeaderIconButton::new(
            ElementId::NamedChild(std::sync::Arc::new(self.id.clone()), "left".into()),
            self.left_system_image,
            move |window, cx| on_left(window, cx),
        )
        .font_size(15.0)
        .guide_anchor(self.left_guide_anchor);

        let right_text = self.right_text.unwrap_or_default();
        let right_enabled = self.right_enabled;
        let on_right = self.on_right.clone();
        let right = LiquidGlassButton::new(
            ElementId::NamedChild(std::sync::Arc::new(self.id.clone()), "right".into()),
            LiquidGlassButtonShape::Capsule,
        )
        .horizontal_padding(14.0)
        .disabled(!right_enabled)
        .accessibility_label(Some(right_text.to_string()))
        .on_press(move |window, cx| {
            if right_enabled {
                on_right(window, cx);
            }
        })
        .child(MainText::new(right_text).px(16.0).font_weight(FontWeight::SEMIBOLD).color(text_primary).number_of_lines(1));

        let title = MainText::new(self.title)
            .px(17.0)
            .font_weight(FontWeight::SEMIBOLD)
            .color(text_primary)
            .number_of_lines(1)
            .flex_shrink(1.0)
            .min_w(px(0.0))
            .mx(px(12.0));

        hstack(0.0)
            .w_full()
            .child(div().flex_1().flex().flex_row().justify_start().child(div().flex_none().child(left)))
            .child(title)
            .child(
                div().flex_1().flex().flex_row().justify_end().child(
                    div()
                        .flex_none()
                        .opacity(if right_enabled { 1.0 } else { DISABLED_OPACITY })
                        .child(right)
                        .guide_anchor(self.right_guide_anchor),
                ),
            )
            .px(px(14.0))
            .py(px(10.0))
    }
}
