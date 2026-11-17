use std::rc::Rc;

use gpui::{
    AnyElement, App, ElementId, Hsla, IntoElement, ParentElement, RenderOnce, SharedString,
    Styled, Window, div, px,
};
use rise_i18n::i18n::tr;
use rise_theme::theme::theme_store::ThemeStore;

use crate::icon_ui::IconUi;
use crate::liquid_chrome::LiquidChromeModifiers;
use crate::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use crate::unread_count_badge_ui::UnreadCountBadgeUi;

/// sizes every page header shares
pub struct PageHeaderMetrics;

impl PageHeaderMetrics {
    pub const TOOLBAR_BUTTON_SIZE: f32 = 45.0;
    pub const BAR_HEIGHT: f32 = 56.0;
    pub const TOP_INSET: f32 = 6.0;
    pub const HEIGHT: f32 = Self::TOP_INSET + Self::BAR_HEIGHT;
}

/// what a header button runs on press
pub type PageHeaderAction = Rc<dyn Fn(&mut Window, &mut App)>;

/// round glass button of the header holding one symbol
#[derive(IntoElement)]
pub struct PageHeaderIconButton {
    id: ElementId,
    system_image: SharedString,
    font_size: f32,
    badge_count: i64,
    icon_color: Option<Hsla>,
    disabled: bool,
    action: PageHeaderAction,
}

impl PageHeaderIconButton {
    pub fn new(
        id: impl Into<ElementId>,
        system_image: impl Into<SharedString>,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            system_image: system_image.into(),
            font_size: 17.0,
            badge_count: 0,
            icon_color: None,
            disabled: false,
            action: Rc::new(action),
        }
    }

    pub fn font_size(mut self, font_size: f32) -> Self {
        self.font_size = font_size;
        self
    }

    pub fn badge_count(mut self, badge_count: i64) -> Self {
        self.badge_count = badge_count;
        self
    }

    pub fn icon_color(mut self, icon_color: impl Into<Option<Hsla>>) -> Self {
        self.icon_color = icon_color.into();
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// the app own word for the common header glyphs
    /// given to the screen reader so it doesnt name an unlabelled symbol in the device language
    pub fn accessibility_label(&self) -> Option<String> {
        Self::glyph_label(&self.system_image)
    }

    fn glyph_label(system_image: &str) -> Option<String> {
        match system_image {
            "xmark" => Some(tr("close")),
            "chevron.backward" | "chevron.left" => Some(tr("back")),
            "checkmark" => Some(tr("done")),
            "ellipsis" => Some(tr("more")),
            _ => None,
        }
    }

    fn glyph(&self, color: Hsla) -> AnyElement {
        let side = px(self.font_size);
        match IconUi::sized(&self.system_image, side, color, false) {
            Some(glyph) => glyph.into_any_element(),
            None => div().size(side).into_any_element(),
        }
    }
}

impl RenderOnce for PageHeaderIconButton {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let text_primary = ThemeStore::shared(cx).read(cx).current_theme().text.primary;
        let glyph = self.glyph(self.icon_color.unwrap_or(text_primary));
        let action = self.action.clone();
        let button = LiquidGlassButton::new(self.id.clone(), LiquidGlassButtonShape::Circle)
            .size(PageHeaderMetrics::TOOLBAR_BUTTON_SIZE)
            .disabled(self.disabled)
            .accessibility_label(self.accessibility_label())
            .on_press(move |window, cx| action(window, cx))
            .child(glyph);

        div().relative().child(button).child(
            div()
                .absolute()
                .top(px(-4.0))
                .left(px(-4.0))
                .child(UnreadCountBadgeUi::new(self.badge_count)),
        )
    }
}

#[derive(IntoElement)]
pub struct PageHeaderBackButton {
    id: ElementId,
    action: PageHeaderAction,
}

impl PageHeaderBackButton {
    pub fn new(id: impl Into<ElementId>, action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self {
            id: id.into(),
            action: Rc::new(action),
        }
    }

    pub(crate) fn from_action(id: impl Into<ElementId>, action: PageHeaderAction) -> Self {
        Self {
            id: id.into(),
            action,
        }
    }
}

impl RenderOnce for PageHeaderBackButton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let action = self.action;
        PageHeaderIconButton::new(self.id, "chevron.backward", move |window, cx| {
            action(window, cx)
        })
        .font_size(16.0)
    }
}

/// round glass slot of the header for content that is not a plain symbol button
#[derive(IntoElement)]
pub struct PageHeaderToolbarSlot {
    size: f32,
    content: AnyElement,
}

impl PageHeaderToolbarSlot {
    pub fn new(content: impl IntoElement) -> Self {
        Self {
            size: PageHeaderMetrics::TOOLBAR_BUTTON_SIZE,
            content: content.into_any_element(),
        }
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }
}

impl RenderOnce for PageHeaderToolbarSlot {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .size(px(self.size))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .child(self.content)
            .liquid_chrome_circle(None)
    }
}
