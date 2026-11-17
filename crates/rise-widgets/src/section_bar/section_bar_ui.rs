use std::rc::Rc;

use gpui::{
    AnyElement, App, ClickEvent, ElementId, Hsla, IntoElement, Pixels, RenderOnce, SharedString,
    Window, div, prelude::*,
};
use rise_theme::AppTheme;
use rise_ui::IconUi;

pub type SectionBarGlyph = Rc<dyn Fn(Pixels, bool, &mut Window, &mut App) -> AnyElement>;

pub type SectionBarSelect = Rc<dyn Fn(&SharedString, &mut Window, &mut App)>;

#[derive(Clone)]
pub enum SectionBarIcon {
    Symbol(SharedString),
    Custom(SectionBarGlyph),
}

/// one button of the section bar, a main tab or a more entry
/// same order and glyph
#[derive(Clone)]
pub struct SectionBarItem {
    pub id: SharedString,
    pub icon: SectionBarIcon,
    pub title: SharedString,
    pub badge: Option<SharedString>,
}

impl SectionBarItem {
    pub fn symbol(
        id: impl Into<SharedString>,
        symbol: impl Into<SharedString>,
        title: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            icon: SectionBarIcon::Symbol(symbol.into()),
            title: title.into(),
            badge: None,
        }
    }

    pub fn custom(
        id: impl Into<SharedString>,
        glyph: SectionBarGlyph,
        title: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            icon: SectionBarIcon::Custom(glyph),
            title: title.into(),
            badge: None,
        }
    }

    pub fn badge(mut self, badge: Option<SharedString>) -> Self {
        self.badge = badge;
        self
    }
}

#[derive(IntoElement)]
pub struct SectionBarUi {
    id: ElementId,
    sections: Vec<SectionBarItem>,
    footer: Vec<SectionBarItem>,
    selected: Option<SharedString>,
    on_select: Option<SectionBarSelect>,
}

impl SectionBarUi {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            sections: Vec::new(),
            footer: Vec::new(),
            selected: None,
            on_select: None,
        }
    }

    pub fn sections(mut self, sections: Vec<SectionBarItem>) -> Self {
        self.sections = sections;
        self
    }

    pub fn footer(mut self, footer: Vec<SectionBarItem>) -> Self {
        self.footer = footer;
        self
    }

    pub fn selected(mut self, selected: Option<SharedString>) -> Self {
        self.selected = selected;
        self
    }

    /// called with the item id on every press, also on the selected one
    /// reselecting scrolls the section to its top and that decision is the caller
    pub fn on_select(
        mut self,
        on_select: impl Fn(&SharedString, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_select = Some(Rc::new(on_select));
        self
    }

    fn item(
        item: SectionBarItem,
        is_selected: bool,
        on_select: Option<SectionBarSelect>,
        theme: &AppTheme,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let metrics = &theme.shell;
        let tint: Hsla = if is_selected {
            theme.primary._100
        } else {
            theme.text.secondary
        };

        let glyph = match &item.icon {
            SectionBarIcon::Symbol(symbol) => {
                IconUi::sized(symbol.as_ref(), metrics.rail_icon_size, tint, is_selected)
                    .map(IntoElement::into_any_element)
            }
            SectionBarIcon::Custom(draw) => {
                Some(draw(metrics.rail_icon_size, is_selected, window, cx))
            }
        };

        let badge = item.badge.clone().map(|text| {
            div()
                .absolute()
                .top_0()
                .right(metrics.rail_padding)
                .min_w(metrics.rail_label_size * 1.6)
                .h(metrics.rail_label_size * 1.6)
                .px(metrics.rail_label_size * 0.4)
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme.semantic.error_100)
                .text_color(gpui::white())
                .text_size(metrics.rail_label_size)
                .child(text)
        });

        let id = item.id.clone();
        div()
            .id(ElementId::Name(item.id.clone()))
            .relative()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .flex_shrink_0()
            .w(metrics.rail_width - metrics.rail_padding * 2.0)
            .h(metrics.rail_item_size)
            .gap(metrics.rail_item_gap / 3.0)
            .rounded(theme.radius._100)
            .cursor_pointer()
            .when(is_selected, |button| button.bg(theme.bg._200))
            .hover(|button| button.bg(theme.bg._200))
            .children(glyph)
            .child(
                div()
                    .max_w_full()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .text_size(metrics.rail_label_size)
                    .text_color(tint)
                    .child(item.title.clone()),
            )
            .children(badge)
            .on_click(move |_: &ClickEvent, window, cx| {
                if let Some(on_select) = &on_select {
                    on_select(&id, window, cx);
                }
            })
            .into_any_element()
    }
}

impl RenderOnce for SectionBarUi {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let metrics = theme.shell;
        let selected = self.selected.clone();
        let on_select = self.on_select.clone();

        let mut sections = Vec::with_capacity(self.sections.len());
        for item in self.sections {
            let is_selected = selected.as_ref() == Some(&item.id);
            sections.push(Self::item(item, is_selected, on_select.clone(), &theme, window, cx));
        }

        let mut footer = Vec::with_capacity(self.footer.len());
        for item in self.footer {
            let is_selected = selected.as_ref() == Some(&item.id);
            footer.push(Self::item(item, is_selected, on_select.clone(), &theme, window, cx));
        }

        div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .h_full()
            .w(metrics.rail_width)
            .bg(theme.bg._100)
            .child(div().flex_shrink_0().h(metrics.window_drag_height))
            .child(
                div()
                    .id(self.id)
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(metrics.rail_item_gap)
                    .px(metrics.rail_padding)
                    .children(sections),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_shrink_0()
                    .items_center()
                    .gap(metrics.rail_item_gap)
                    .p(metrics.rail_padding)
                    .children(footer),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rise_theme::ShellMetrics;

    #[test]
    fn an_item_keeps_the_symbol_ios_declares() {
        let item = SectionBarItem::symbol("chats", "bubble.left.and.bubble.right", "Chats");
        match item.icon {
            SectionBarIcon::Symbol(symbol) => {
                assert_eq!(symbol, SharedString::from("bubble.left.and.bubble.right"));
            }
            SectionBarIcon::Custom(_) => panic!("a symbol item drew a custom glyph"),
        }
        assert_eq!(item.badge, None);
    }

    #[test]
    fn a_badge_is_carried_as_the_caller_formatted_it() {
        let item = SectionBarItem::symbol("chats", "bubble.left.and.bubble.right", "Chats")
            .badge(Some("999+".into()));
        assert_eq!(item.badge, Some(SharedString::from("999+")));
    }

    #[test]
    fn a_button_fits_inside_the_bar() {
        let metrics = ShellMetrics::default();
        assert!(metrics.rail_width - metrics.rail_padding * 2.0 > metrics.rail_icon_size);
    }
}
