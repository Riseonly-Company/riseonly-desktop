use std::rc::Rc;

use gpui::{App, IntoElement, RenderOnce, SharedString, Window, white};
use rise_i18n::{FormatArg, tr, tr_args};
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns, GroupedCardMetrics};
use rise_ui::main_text::MainText;
use rise_ui::swiftui::prelude::*;

use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_shared_ui::ChatProfileEmptyState;
use crate::modules::user::pages::profile::settings::settings_search::settings_search_index::{
    SettingsSearchResult, SettingsSearchTarget,
};

pub type SettingsSearchOpen = Rc<dyn Fn(SettingsSearchTarget, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct SettingsSearchResults {
    results: Rc<Vec<SettingsSearchResult>>,
    on_open: SettingsSearchOpen,
}

impl SettingsSearchResults {
    pub fn new(results: Rc<Vec<SettingsSearchResult>>, on_open: SettingsSearchOpen) -> Self {
        Self { results, on_open }
    }

    fn row(&self, result: &SettingsSearchResult, theme: &AppTheme) -> GroupedBtn {
        let tile = div()
            .size(px(GroupedCardMetrics::ICON_SIDE))
            .flex_none()
            .bg(result.entry.icon_background)
            .continuous_corners(11.0)
            .flex()
            .items_center()
            .justify_center()
            .children(IconUi::sized(result.entry.icon, px(17.0), white(), false));
        let runs = result.highlights.iter().map(|range| (range.clone(), theme.primary._100)).collect();
        let mut texts = vstack(2.0).flex_1().min_w(px(0.0)).align(HorizontalAlignment::Leading);
        if !result.crumb.is_empty() {
            texts = texts
                .child(MainText::new(result.crumb.clone()).px(12.0).color(theme.text.secondary).number_of_lines(1));
        }
        texts = texts.child(
            MainText::new(result.title.clone()).px(15.0).color(theme.text.primary).color_runs(runs).number_of_lines(1),
        );
        let chevron = IconUi::sized(IconUi::trailing_chevron(), px(12.0), theme.text.secondary.opacity(0.75), false);
        let (on_open, target) = (self.on_open.clone(), result.entry.target.clone());
        let content = plain(SharedString::from(format!("settings-search-{}", result.entry.id)))
            .w_full()
            .on_press(move |window: &mut Window, cx: &mut App| on_open(target.clone(), window, cx))
            .child(
                hstack(GroupedCardMetrics::ICON_SPACING)
                    .w_full()
                    .child(tile)
                    .child(texts)
                    .children(chevron.map(|chevron| div().flex_none().child(chevron)))
                    .padding_edges(EdgeSet::HORIZONTAL, GroupedCardMetrics::ROW_HORIZONTAL_PADDING)
                    .padding_edges(EdgeSet::VERTICAL, GroupedCardMetrics::ROW_VERTICAL_PADDING),
            );
        GroupedBtn::new(SharedString::from(result.entry.id.clone()), result.title.clone()).content_slot(content)
    }
}

impl RenderOnce for SettingsSearchResults {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let Some(best) = self.results.first() else {
            return div()
                .w_full()
                .pt(px(40.0))
                .child(ChatProfileEmptyState::new("magnifyingglass", tr("settings_search_empty_title")))
                .into_any_element();
        };
        let mut column = vstack(18.0).w_full().child(
            GroupedBtns::new("settings-search-best", vec![self.row(best, &theme)])
                .title(tr("settings_search_best_match").to_uppercase())
                .uses_lazy_rows(false),
        );
        let rest: Vec<GroupedBtn> = self.results.iter().skip(1).map(|result| self.row(result, &theme)).collect();
        if !rest.is_empty() {
            let title = tr_args("settings_search_more_count", &[FormatArg::Int(rest.len() as i64)]).to_uppercase();
            column = column.child(GroupedBtns::new("settings-search-more", rest).title(title));
        }
        column.into_any_element()
    }
}
