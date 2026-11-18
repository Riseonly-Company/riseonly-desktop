use gpui::{App, Context, Entity, Subscription, Window};
use rise_i18n::tr;
use rise_platform::haptics::selection_changed;
use rise_theme::{AppTheme, RiseThemeAppearance, ThemeStore};
use rise_ui::grouped_btns::GroupedCardMetrics;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::app::router::navigation_types::MainTab;
use crate::core::widgets::navigations::main_tab_bar::MainTabBar;
use crate::core::widgets::navigations::main_tab_bar_buttons::RailEntry;
use crate::core::widgets::navigations::main_tab_bar_settings::{RiseTabBarPlatform, RiseTabBarSettings};
use crate::core::widgets::wrappers::list_shell_ui::{
    ListShellScrollUnderChromeModifiers, SettingsWrapperUi, page_column_backdrop,
};
use crate::modules::user::engine::rise_user_tab_bar_presentation::RiseUserTabBarPresentation;
use crate::modules::user::pages::profile::settings::customization_settings::settings::tab_bar_settings::tab_bar_order_group::TabBarOrderGroup;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;

struct TabBarSettingsState {
    platform: RiseTabBarPlatform,
    preview_tab: MainTab,
    _subscriptions: Vec<Subscription>,
}

impl TabBarSettingsState {
    fn new(cx: &mut Context<Self>) -> Self {
        let theme_store = ThemeStore::shared(cx);
        let presentation = RiseUserTabBarPresentation::shared(cx);
        Self {
            platform: RiseTabBarPlatform::CURRENT,
            preview_tab: MainTab::Posts,
            _subscriptions: vec![
                cx.observe(&theme_store, |_, _, cx| cx.notify()),
                cx.observe(&presentation, |_, _, cx| cx.notify()),
            ],
        }
    }
}

fn presented_settings(platform: RiseTabBarPlatform, cx: &mut App) -> RiseTabBarSettings {
    RiseUserTabBarPresentation::shared(cx).read(cx).settings()[platform]
}

fn change(state: &Entity<TabBarSettingsState>, edit: impl FnOnce(&mut RiseTabBarSettings), cx: &mut App) {
    let platform = state.read(cx).platform;
    let mut next = presented_settings(platform, cx);
    edit(&mut next);
    selection_changed();
    UserInteractionsStore::shared(cx).update(cx, |store, cx| store.set_tab_bar_settings(next, platform, cx));
}

fn preview(state: &Entity<TabBarSettingsState>, settings: RiseTabBarSettings, theme: &AppTheme) -> impl IntoElement {
    let current_tab = Binding::entity(
        state.clone(),
        |state: &TabBarSettingsState| state.preview_tab,
        |state: &mut TabBarSettingsState, tab, _| state.preview_tab = tab,
    );
    let radius = theme.shell.block_radius;
    vstack(8.0)
        .flex_none()
        .align(HorizontalAlignment::Leading)
        .pl(px(GroupedCardMetrics::HORIZONTAL_PADDING))
        .child(MainText::new(tr("tab_bar_preview_title").to_uppercase()).px(12.0).color(theme.text.secondary))
        .child(
            div()
                .relative()
                .flex_none()
                .w(theme.shell.rail_width)
                .h(px(MainTabBar::preview_height()))
                .rounded(radius)
                .bg(theme.bg._100)
                .child(MainTabBar::new(current_tab, settings).is_preview(true).corner_radius(radius))
                .child(centred_stroke(theme.border._200.opacity(0.6), 0.5, Shape::continuous(f32::from(radius)))),
        )
}

#[derive(IntoElement)]
pub struct TabBarSettings;

impl TabBarSettings {
    pub fn new() -> Self {
        Self
    }
}

impl RenderOnce for TabBarSettings {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state = window.use_keyed_state("tab-bar-settings-state", cx, |_, cx| TabBarSettingsState::new(cx));
        let nav_store = env::<NavigationStoreKey>(cx);
        let appearance = ThemeStore::shared(cx).read(cx).resolved_appearance();
        let (surface, background) = if appearance == RiseThemeAppearance::Light {
            (theme.bg._000, theme.bg._600)
        } else {
            (theme.bg._200, theme.bg._100)
        };
        let platform = state.read(cx).platform;
        let shown = presented_settings(platform, cx);

        let order_group = TabBarOrderGroup::new(shown, surface, move |order: Vec<RailEntry>, cx| {
            RailEntry::save(&order, cx);
        })
        .hint(tr("desktop_tab_bar_order_hint"));

        page_column_backdrop(background, cx);

        SettingsWrapperUi::new(tr("settings_tab_bar"), move |_, cx| {
            if let Some(nav_store) = &nav_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .content(
            scroll_view("tab-bar-settings-scroll")
                .size_full()
                .bg(background)
                .list_shell_scroll_under_chrome(cx)
                .child(
                    vstack(20.0)
                        .w_full()
                        .pt(px(12.0))
                        .child(
                            hstack(0.0)
                                .w_full()
                                .items_start()
                                .child(preview(&state, shown, &theme))
                                .child(div().flex_1().min_w_0().child(order_group)),
                        )
                        .child(div().flex_none().h(px(80.0))),
                ),
        )
        .on_appear("tab-bar-settings-appear", |_, cx| {
            UserInteractionsStore::shared(cx).update(cx, |store, cx| store.load_tab_bar_settings(false, cx));
        })
    }
}
