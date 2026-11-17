use gpui::{App, ElementId, FontWeight, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div, px};
use rise_theme::ThemeStore;

use crate::IconUi;
use crate::context_menu_ui::{ContextMenuItem, ContextMenuUi};
use crate::liquid_chrome::LiquidChromeModifiers;
use crate::swiftui::font::{Font, SwiftUiFont};
use crate::swiftui::frame::{EdgeSet, SwiftUiFrame};
use crate::swiftui::stacks::hstack;

const CHEVRON_SYMBOL: &str = "chevron.down";

/// a capsule showing the current filter, opens the filter menu
#[derive(IntoElement)]
pub struct FilterPillUi {
    id: ElementId,
    items: Vec<ContextMenuItem>,
    selected_key: String,
    current_label: SharedString,
}

impl FilterPillUi {
    pub fn new(
        id: impl Into<ElementId>,
        items: Vec<ContextMenuItem>,
        selected_key: impl Into<String>,
        current_label: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            items,
            selected_key: selected_key.into(),
            current_label: current_label.into(),
        }
    }
}

impl RenderOnce for FilterPillUi {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (text_primary, text_secondary) = {
            let store = ThemeStore::shared(cx);
            let theme = store.read(cx).current_theme();
            (theme.text.primary, theme.text.secondary)
        };
        let label = hstack(4.0)
            .child(
                div()
                    .swiftui_font(Font::system_weight(13.0, FontWeight::SEMIBOLD))
                    .text_color(text_primary)
                    .child(self.current_label),
            )
            .children(IconUi::sized(CHEVRON_SYMBOL, px(11.0), text_secondary, false))
            .padding_edges(EdgeSet::HORIZONTAL, 12.0)
            .padding_edges(EdgeSet::VERTICAL, 7.0)
            .liquid_chrome_capsule();
        ContextMenuUi::new(self.id, self.items, label).selected(self.selected_key)
    }
}
