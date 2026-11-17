use std::rc::Rc;

use gpui::{App, ElementId, Hsla, IntoElement, RenderOnce, Window, div, prelude::*, px};
use rise_theme::ThemeStore;
use rise_ui::icon_ui::IconUi;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::swiftui::button_style::plain;

pub type ComposerSendButtonAction = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct ComposerSendButton {
    id: ElementId,
    pub size: f32,
    pub is_editing: bool,
    pub is_enabled: bool,
    pub uses_liquid_glass: bool,
    action: ComposerSendButtonAction,
}

impl ComposerSendButton {
    /// defaults are size 38, not editing
    /// enabled, liquid glass on
    pub fn new(id: impl Into<ElementId>, action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self {
            id: id.into(),
            size: 38.,
            is_editing: false,
            is_enabled: true,
            uses_liquid_glass: true,
            action: Rc::new(action),
        }
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    pub fn is_editing(mut self, is_editing: bool) -> Self {
        self.is_editing = is_editing;
        self
    }

    pub fn is_enabled(mut self, is_enabled: bool) -> Self {
        self.is_enabled = is_enabled;
        self
    }

    pub fn uses_liquid_glass(mut self, uses_liquid_glass: bool) -> Self {
        self.uses_liquid_glass = uses_liquid_glass;
        self
    }

    fn icon(&self, primary: Hsla, secondary: Hsla) -> impl IntoElement + use<> {
        let (symbol, side, is_filled) = if self.is_editing {
            ("checkmark", 18., false)
        } else {
            ("paperplane.fill", 17., true)
        };
        let color = if self.is_enabled { primary } else { secondary };
        div()
            .pl(px(if self.is_editing { 0. } else { 2. }))
            .children(IconUi::sized(symbol, px(side), color, is_filled))
    }
}

impl RenderOnce for ComposerSendButton {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (primary, secondary, background) = {
            let store = ThemeStore::shared(cx);
            let theme = store.read(cx).current_theme();
            (theme.primary._100, theme.text.secondary, theme.bg._100)
        };
        let icon = self.icon(primary, secondary);
        let action = self.action.clone();
        if self.uses_liquid_glass {
            LiquidGlassButton::new(self.id, LiquidGlassButtonShape::Circle)
                .size(self.size)
                .disabled(!self.is_enabled)
                .on_press(move |window, cx| action(window, cx))
                .child(icon)
                .into_any_element()
        } else {
            let is_enabled = self.is_enabled;
            plain(self.id)
                .on_press(move |window, cx| {
                    if is_enabled {
                        action(window, cx);
                    }
                })
                .opacity(if is_enabled { 1. } else { 0.55 })
                .child(
                    div()
                        .size(px(self.size))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(background)
                        .child(icon),
                )
                .into_any_element()
        }
    }
}
