use gpui::{AnyElement, App, Context, Entity, Hsla, Subscription, Window};
use rise_canon::RiseGuideAnchor;
use rise_i18n::tr;
use rise_platform::haptics::selection_changed;
use rise_theme::{AppTheme, RiseThemeAppearance, RiseThemePlatform, RiseThemePreferences, ThemeStore, color};
use rise_ui::IconUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::widgets::wrappers::list_shell_ui::{
    ListShellScrollUnderChromeModifiers, SettingsWrapperUi, page_column_backdrop,
};
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::theme::stores::theme_presets::theme_presets_actions::theme_presets_actions::ThemePresetsActionsStore;
use crate::modules::theme::stores::theme_presets::theme_presets_services::theme_presets_services::{
    ThemePresetsLoadState, ThemePresetsServicesStore,
};
use crate::modules::user::engine::rise_user_presentation::RiseUserPresentation;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

/// selected platform and the observed stores, changes re-render the page
struct ThemeSettingsState {
    selected_platform: RiseThemePlatform,
    _subscriptions: Vec<Subscription>,
}

impl ThemeSettingsState {
    fn new(cx: &mut Context<Self>) -> Self {
        let theme_store = ThemeStore::shared(cx);
        let presentation = RiseUserPresentation::shared(cx);
        let sandbox = GuideSandbox::shared(cx);
        Self {
            selected_platform: RiseThemePlatform::THIS_DEVICE,
            _subscriptions: vec![
                cx.observe(&theme_store, |_, _, cx| cx.notify()),
                cx.observe(&presentation, |_, _, cx| cx.notify()),
                cx.observe(&sandbox, |_, _, cx| cx.notify()),
            ],
        }
    }
}

fn platform_raw_value(platform: RiseThemePlatform) -> &'static str {
    match platform {
        RiseThemePlatform::Mobile => "mobile",
        RiseThemePlatform::Desktop => "desktop",
        RiseThemePlatform::Web => "web",
    }
}

fn appearance_label(appearance: Option<RiseThemeAppearance>) -> String {
    match appearance {
        Some(RiseThemeAppearance::Dark) => tr("theme_dark"),
        Some(RiseThemeAppearance::Light) => tr("theme_light"),
        None => tr("theme_system"),
    }
}

fn set_appearance(appearance: Option<RiseThemeAppearance>, platform: RiseThemePlatform, cx: &mut App) {
    selection_changed();
    UserInteractionsStore::shared(cx).update(cx, |store, cx| store.set_theme_appearance(appearance, platform, cx));
    if platform != RiseThemePlatform::THIS_DEVICE || GuideSandbox::shared(cx).read(cx).owns_theme_scene() {
        return;
    }
    let resolved = ThemeStore::shared(cx).read(cx).resolved_appearance();
    ThemePresetsActionsStore::shared(cx).update(cx, |actions, cx| actions.appearance_did_change_action(resolved, cx));
}

fn selection_indicator(selected: bool, theme: &AppTheme) -> impl IntoElement {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(29.0))
        .children(IconUi::sized(
            if selected { "checkmark.circle.fill" } else { "circle" },
            px(21.0),
            if selected { theme.primary._100 } else { theme.border._300 },
            false,
        ))
}

fn mutation_progress(
    platform: RiseThemePlatform,
    appearance: Option<RiseThemeAppearance>,
    theme: &AppTheme,
    cx: &mut App,
) -> Option<AnyElement> {
    let services = UserServicesStore::shared(cx);
    let services = services.read(cx);
    if !services.theme_settings_is_mutating(platform, cx) || services.theme_preferences(cx).appearance(platform) != appearance {
        return None;
    }
    Some(progress_view().control_size(ControlSize::Small).tint(theme.primary._100).into_any_element())
}

#[derive(IntoElement)]
pub struct ThemeSettings;

impl ThemeSettings {
    pub fn new() -> Self {
        Self
    }

    fn grouped_page_background(appearance: RiseThemeAppearance, theme: &AppTheme) -> Hsla {
        if appearance == RiseThemeAppearance::Light { theme.bg._600 } else { theme.bg._100 }
    }

    fn grouped_surface(appearance: RiseThemeAppearance, theme: &AppTheme) -> Hsla {
        if appearance == RiseThemeAppearance::Light { theme.bg._000 } else { theme.bg._200 }
    }

    #[allow(clippy::too_many_arguments)]
    fn platform_row(
        platform: RiseThemePlatform,
        icon: &'static str,
        tile: &'static str,
        title: String,
        state: &Entity<ThemeSettingsState>,
        preferences: RiseThemePreferences,
        theme: &AppTheme,
        cx: &mut App,
    ) -> GroupedBtn {
        let selected = state.read(cx).selected_platform == platform;
        let label = appearance_label(preferences.appearance(platform));
        let state = state.clone();
        GroupedBtn::new(format!("theme-platform-{}", platform_raw_value(platform)), title)
            .icon(icon)
            .icon_background(color(tile))
            .text_line_limit(2)
            .accessibility_value(label.clone())
            .accessibility_selected(selected)
            .show_chevron(false)
            .trailing_keeps_width(true)
            .trailing_slot(
                hstack(8.0)
                    .flex_none()
                    .child(div().flex_none().child(MainText::new(label).px(13.0).color(theme.text.secondary).number_of_lines(1)))
                    .child(div().flex_none().size(px(18.0)).children(IconUi::sized(
                        if selected { "checkmark.circle.fill" } else { "circle" },
                        px(18.0),
                        if selected { theme.primary._100 } else { theme.border._300 },
                        false,
                    ))),
            )
            .callback(move |_, cx| {
                state.update(cx, |state, cx| {
                    state.selected_platform = platform;
                    cx.notify();
                });
            })
    }

    fn platform_rows(
        state: &Entity<ThemeSettingsState>,
        preferences: RiseThemePreferences,
        theme: &AppTheme,
        cx: &mut App,
    ) -> Vec<GroupedBtn> {
        vec![
            Self::platform_row(
                RiseThemePlatform::Mobile,
                "iphone",
                "#0088FF",
                tr("theme_platform_mobile"),
                state,
                preferences,
                theme,
                cx,
            ),
            Self::platform_row(
                RiseThemePlatform::Desktop,
                "desktopcomputer",
                "#6C5CE7",
                tr("theme_platform_desktop"),
                state,
                preferences,
                theme,
                cx,
            ),
            Self::platform_row(
                RiseThemePlatform::Web,
                "globe",
                "#00A9BB",
                tr("theme_platform_web"),
                state,
                preferences,
                theme,
                cx,
            ),
        ]
    }

    fn appearance_row(
        appearance: RiseThemeAppearance,
        selected: bool,
        title: String,
        selected_platform: RiseThemePlatform,
        theme: &AppTheme,
        cx: &mut App,
    ) -> GroupedBtn {
        let mut row = GroupedBtn::new(format!("theme-appearance-{}", appearance.raw_value()), title)
            .leading_slot(selection_indicator(selected, theme))
            .text_line_limit(1)
            .accessibility_selected(selected)
            .show_chevron(false)
            .callback(move |_, cx| set_appearance(Some(appearance), selected_platform, cx));
        if let Some(progress) = mutation_progress(selected_platform, Some(appearance), theme, cx) {
            row = row.trailing_slot(progress);
        }
        row
    }

    fn appearance_rows(
        selected_platform: RiseThemePlatform,
        preferences: RiseThemePreferences,
        theme: &AppTheme,
        cx: &mut App,
    ) -> Vec<GroupedBtn> {
        let selected = preferences.appearance(selected_platform);
        vec![
            Self::appearance_row(
                RiseThemeAppearance::Dark,
                selected == Some(RiseThemeAppearance::Dark),
                tr("theme_dark_full"),
                selected_platform,
                theme,
                cx,
            ),
            Self::appearance_row(
                RiseThemeAppearance::Light,
                selected == Some(RiseThemeAppearance::Light),
                tr("theme_light_full"),
                selected_platform,
                theme,
                cx,
            ),
        ]
    }

    fn system_row(
        selected_platform: RiseThemePlatform,
        preferences: RiseThemePreferences,
        theme: &AppTheme,
        cx: &mut App,
    ) -> GroupedBtn {
        let follows_system = preferences.appearance(selected_platform).is_none();
        let mut row = GroupedBtn::new("theme-system", tr("theme_follow_system"))
            .leading_slot(selection_indicator(follows_system, theme))
            .text_line_limit(1)
            .accessibility_selected(follows_system)
            .show_chevron(false)
            .callback(move |_, cx| set_appearance(None, selected_platform, cx));
        if let Some(progress) = mutation_progress(selected_platform, None, theme, cx) {
            row = row.trailing_slot(progress);
        }
        row
    }

    fn theme_settings_sync_group(surface: Hsla, theme: &AppTheme) -> impl IntoElement {
        GroupedBtns::new(
            "theme-sync-group",
            vec![
                GroupedBtn::new("theme-retry", tr("common_retry"))
                    .icon("arrow.triangle.2.circlepath")
                    .icon_background(theme.primary._100)
                    .show_chevron(false)
                    .callback(|_, cx| {
                        UserInteractionsStore::shared(cx).update(cx, |store, cx| store.load_theme_settings(true, cx));
                    }),
            ],
        )
        .end_group_title(tr("theme_settings_sync_error"))
        .surface_color(surface)
    }
}

impl RenderOnce for ThemeSettings {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state = window.use_keyed_state("theme-settings-state", cx, |_, cx| ThemeSettingsState::new(cx));
        let nav_store = env::<NavigationStoreKey>(cx);
        let selected_platform = state.read(cx).selected_platform;
        let theme_store = ThemeStore::shared(cx);
        let appearance = theme_store.read(cx).resolved_appearance();
        let surface = Self::grouped_surface(appearance, &theme);
        let background = Self::grouped_page_background(appearance, &theme);

        let services = UserServicesStore::shared(cx);
        let owns_theme_scene = GuideSandbox::shared(cx).read(cx).owns_theme_scene();
        let has_value = services.read(cx).theme_settings_has_value(cx);
        let preferences = if has_value && !owns_theme_scene {
            services.read(cx).theme_preferences(cx)
        } else {
            theme_store.read(cx).preferences()
        };
        let has_sync_error = services.read(cx).theme_settings_error(cx).is_some();

        let platform_rows = Self::platform_rows(&state, preferences, &theme, cx);
        let appearance_rows = Self::appearance_rows(selected_platform, preferences, &theme, cx);
        let system_row = Self::system_row(selected_platform, preferences, &theme, cx);

        page_column_backdrop(background, cx);

        SettingsWrapperUi::new(tr("settings_customization_your_theme"), move |_, cx| {
            if let Some(nav_store) = &nav_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .content(
            scroll_view("theme-settings-scroll")
                .size_full()
                .bg(background)
                .list_shell_scroll_under_chrome(cx)
                .child(
                    vstack(20.0)
                        .w_full()
                        .pt(px(12.0))
                        .child(
                            GroupedBtns::new("theme-platforms-group", platform_rows)
                                .title(tr("theme_platforms_title").to_uppercase())
                                .surface_color(surface)
                                .guide_anchor(RiseGuideAnchor::ThemePlatforms),
                        )
                        .child(
                            GroupedBtns::new("theme-appearance-group", appearance_rows)
                                .title(tr("theme_choice_title").to_uppercase())
                                .surface_color(surface)
                                .guide_anchor(RiseGuideAnchor::ThemeAppearance),
                        )
                        .child(
                            GroupedBtns::new("theme-system-group", vec![system_row])
                                .end_group_title(tr("theme_follow_system_hint"))
                                .surface_color(surface),
                        )
                        .when(has_sync_error, |stack| {
                            stack.child(Self::theme_settings_sync_group(surface, &theme))
                        })
                        .child(div().flex_none().h(px(80.0))),
                ),
        )
        .on_appear("theme-settings-appear", |_, cx| {
            UserInteractionsStore::shared(cx).update(cx, |store, cx| store.load_theme_settings(false, cx));
            if *ThemePresetsServicesStore::shared(cx).read(cx).load_state() == ThemePresetsLoadState::Idle {
                ThemePresetsActionsStore::shared(cx).update(cx, |actions, cx| actions.load_action(cx));
            }
        })
    }
}
