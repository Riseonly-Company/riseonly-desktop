use gpui::{App, Context, Entity, SharedString, Subscription, Window};
use rise_i18n::tr;
use rise_ui::flag_icon_ui::FlagIconUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns, GroupedCardMetrics};
use rise_ui::scroll_chrome_shadow_ui::ScrollChromeShadowStyle;
use rise_ui::search_field_ui::SearchFieldUi;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, ListShellUi};
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_shared_ui::ChatProfileEmptyState;
use crate::modules::user::engine::rise_user_locale_presentation::RiseUserLocalePresentation;
use crate::modules::user::stores::user_locale::user_locale_interactions::user_locale_interactions::UserLocaleInteractionsStore;
use crate::modules::user::stores::user_locale::user_locale_services::user_locale_services::UserLocaleServicesStore;

const SELECTED_MARK: &str = "\u{2713}";

/// search query and services store, changes re-render the page
struct RegionSettingsState {
    query: String,
    _subscriptions: Vec<Subscription>,
}

impl RegionSettingsState {
    fn new(cx: &mut Context<Self>) -> Self {
        let presentation = RiseUserLocalePresentation::shared(cx);
        Self {
            query: String::new(),
            _subscriptions: vec![cx.observe(&presentation, |_, _, cx| cx.notify())],
        }
    }
}

#[derive(IntoElement)]
pub struct RegionSettings;

impl RegionSettings {
    pub fn new() -> Self {
        Self
    }

    fn rows(state: &Entity<RegionSettingsState>, cx: &mut App) -> Vec<GroupedBtn> {
        let query = state.read(cx).query.clone();
        let services = UserLocaleServicesStore::shared(cx);
        let (rows, selected) =
            services.update(cx, |services, cx| (services.regions(&query, cx), services.effective_region(cx)));
        rows.into_iter()
            .map(|region| {
                let code = region.code;
                let is_selected = code == selected;
                let mut row = GroupedBtn::new(code, region.localized_name())
                    .leading_slot(FlagIconUi::new(region.flag()))
                    .accessibility_selected(is_selected)
                    .show_chevron(false)
                    .callback(move |_, cx| {
                        UserLocaleInteractionsStore::shared(cx)
                            .update(cx, |interactions, cx| interactions.select_region(code, cx));
                    });
                if is_selected {
                    row = row.trailing(SELECTED_MARK);
                }
                row
            })
            .collect()
    }
}

impl RenderOnce for RegionSettings {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state = window.use_keyed_state("region-settings-state", cx, |_, cx| RegionSettingsState::new(cx));
        let nav_store = env::<NavigationStoreKey>(cx);
        let rows = Self::rows(&state, cx);

        let content = vstack(14.0)
            .w_full()
            .pt(px(12.0))
            .when(rows.is_empty(), |stack| {
                stack.child(
                    div()
                        .w_full()
                        .pt(px(40.0))
                        .child(ChatProfileEmptyState::new("globe", tr("settings_region_empty_title"))),
                )
            })
            .when(!rows.is_empty(), |stack| stack.child(GroupedBtns::new("region-settings-rows", rows)))
            .child(div().flex_none().h(px(80.0)));

        ListShellUi::new()
            .title(SharedString::from(tr("settings_region_title")))
            .on_back(move |_, cx| {
                if let Some(nav_store) = &nav_store {
                    nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
                }
            })
            .header_chrome_shadow(Some(ScrollChromeShadowStyle::list_shell_tall_header()))
            .below_header(
                div().w_full().px(px(GroupedCardMetrics::HORIZONTAL_PADDING)).child(
                    SearchFieldUi::new(
                        "region-settings-search",
                        Binding::entity(
                            state,
                            |state: &RegionSettingsState| state.query.clone(),
                            |state: &mut RegionSettingsState, query, _| state.query = query,
                        ),
                    )
                    .placeholder(SharedString::from(tr("settings_region_search_placeholder")))
                    .uses_inline_text_field(true),
                ),
            )
            .content(
                scroll_view("region-settings-scroll")
                    .size_full()
                    .bg(theme.bg._100)
                    .list_shell_scroll_under_chrome(cx)
                    .child(content),
            )
            .on_appear("region-settings-appear", |_, cx| {
                UserLocaleInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.open_region_settings(cx));
            })
    }
}
