use std::ops::RangeInclusive;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, Axis, ElementId, FontWeight, Hsla, SharedString, Window, white};
use rise_theme::theme::theme_store::ThemeStore;
use rise_ui::IconUi;
use rise_ui::grouped_btns::GroupedCardMetrics;
use rise_ui::main_text::ignores_text_size_delta;
use rise_ui::swiftui::prelude::*;

use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::ui::input_field_accessory::InputFieldAccessory;
use crate::core::ui::shaped_text_field::ShapedTextField;

/// settings card row whose label slot holds a text field
/// exists so there is one edit form for profile and chat settings, same heights and insets
#[derive(IntoElement)]
pub struct SettingsFieldRow {
    id: ElementId,
    icon: String,
    icon_background: Hsla,
    placeholder: String,
    text: Binding<String>,
    error_message: Option<String>,
    rule: RiseFieldRule,
    axis: Axis,
    status: Option<SettingsFieldStatus>,
    on_error_tap: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
}

impl SettingsFieldRow {
    const VERTICAL_LINE_LIMIT: RangeInclusive<usize> = 2..=5;
    const SINGLE_LINE_LIMIT: RangeInclusive<usize> = 1..=1;

    const MIN_HEIGHT: f32 = 54.;
    const FIELD_FONT_SIZE: f32 = 15.;

    pub fn new(
        id: impl Into<ElementId>,
        icon: impl Into<String>,
        icon_background: Hsla,
        placeholder: impl Into<String>,
        text: Binding<String>,
    ) -> Self {
        Self {
            id: id.into(),
            icon: icon.into(),
            icon_background,
            placeholder: placeholder.into(),
            text,
            error_message: None,
            rule: RiseFieldRule::NONE,
            axis: Axis::Horizontal,
            status: None,
            on_error_tap: None,
        }
    }

    pub fn error_message(mut self, error_message: Option<String>) -> Self {
        self.error_message = error_message;
        self
    }

    pub fn rule(mut self, rule: RiseFieldRule) -> Self {
        self.rule = rule;
        self
    }

    pub fn axis(mut self, axis: Axis) -> Self {
        self.axis = axis;
        self
    }

    pub fn status(mut self, status: Option<SettingsFieldStatus>) -> Self {
        self.status = status;
        self
    }

    pub fn on_error_tap(mut self, on_error_tap: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_error_tap = Some(Rc::new(on_error_tap));
        self
    }

    fn is_multiline(&self) -> bool {
        self.axis == Axis::Vertical
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }
}

impl RenderOnce for SettingsFieldRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let is_multiline = self.is_multiline();
        let text = self.text.get(cx);
        let remaining = self.rule.remaining(&text);

        let text_delta = if ignores_text_size_delta(cx) { 0. } else { ThemeStore::shared(cx).read(cx).text_delta() };
        let field_font_size = (Self::FIELD_FONT_SIZE + text_delta).max(1.);

        let body = theme.typography.body();
        let body_size = f32::from(body.size);
        let body_line_height = f32::from(body.line_height);
        let first_line_height =
            if body_size > 0. { body_line_height * field_font_size / body_size } else { body_line_height };
        let icon_top = if is_multiline { ((first_line_height - GroupedCardMetrics::ICON_SIDE) / 2.).max(0.) } else { 0. };
        let field_top = if is_multiline { ((GroupedCardMetrics::ICON_SIDE - first_line_height) / 2.).max(0.) } else { 0. };
        let icon = Shape::continuous(8.)
            .round(div())
            .bg(self.icon_background)
            .size(px(GroupedCardMetrics::ICON_SIDE))
            .flex()
            .items_center()
            .justify_center()
            .when(icon_top > 0., |icon| icon.mt(px(icon_top)))
            .children(IconUi::sized(&self.icon, px(14.), white(), false));

        let line_limit = if is_multiline { Self::VERTICAL_LINE_LIMIT } else { Self::SINGLE_LINE_LIMIT };
        let field = div()
            .flex_1()
            .when(field_top > 0., |field| field.mt(px(field_top)))
            .text_color(theme.text.primary)
            .child(
                ShapedTextField::new(self.child_id("field"), self.placeholder.clone(), self.text.clone())
                    .font_size(field_font_size)
                    .axis(self.axis)
                    .max_visible_lines(*line_limit.end()),
            );

        let trailing = match self.status {
            Some(status) if !InputFieldAccessory::is_visible(remaining, self.error_message.as_deref()) => {
                SettingsFieldStatusIcon { status }.into_any_element()
            }
            _ => InputFieldAccessory::new(self.child_id("accessory"), remaining, self.error_message.clone())
                .on_error_tap(self.on_error_tap.clone())
                .into_any_element(),
        };

        hstack(GroupedCardMetrics::ICON_SPACING)
            .align(if is_multiline { VerticalAlignment::Top } else { VerticalAlignment::Center })
            .child(icon)
            .child(field)
            .child(trailing)
            .padding_edges(EdgeSet::HORIZONTAL, GroupedCardMetrics::ROW_HORIZONTAL_PADDING)
            .padding_edges(EdgeSet::VERTICAL, GroupedCardMetrics::ROW_VERTICAL_PADDING)
            .min_h(px(Self::MIN_HEIGHT))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsFieldStatus {
    Checking,
    Accepted,
}

#[derive(IntoElement)]
struct SettingsFieldStatusIcon {
    status: SettingsFieldStatus,
}

impl RenderOnce for SettingsFieldStatusIcon {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let content = match self.status {
            SettingsFieldStatus::Checking => {
                progress_view().control_size(ControlSize::Small).tint(theme.text.secondary).into_any_element()
            }
            SettingsFieldStatus::Accepted => div()
                .swiftui_font(Font::system_weight(17., FontWeight::SEMIBOLD))
                .children(IconUi::sized("checkmark.circle.fill", px(17.), theme.semantic.success_100, true))
                .into_any_element(),
        };
        div().w(px(InputFieldAccessory::WIDTH)).flex().items_center().justify_center().child(content)
    }
}
