use std::ops::RangeInclusive;
use std::rc::Rc;

use gpui::{App, Context, Entity, Hsla, SharedString, Subscription, Window, white};
use rise_canon::RiseTabBarButton;
use rise_i18n::number_format::decimal_number_formatter::DecimalNumberFormatter;
use rise_i18n::number_format::percent_number_formatter::PercentNumberFormatter;
use rise_i18n::{I18n, tr};
use rise_platform::haptics::selection_changed;
use rise_theme::{AppTheme, RiseThemeAppearance, ThemeStore, color};
use rise_ui::IconUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns, GroupedCardMetrics};
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::main_text::MainText;
use rise_ui::settings_icon_palette::SettingsIconPalette;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::slider::slider;
use rise_ui::switch_ui::SwitchUi;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::app::router::navigation_types::MainTab;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWs;
use crate::core::preferences::tab_bar_browse_preferences::TabBarBrowsePreferences;
use crate::core::rise_engine_bridge::rise_transport_status::RiseTransportStatus;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presentation::{
    HoldContextMenuPresentationActions, HoldContextMenuPresentationOnAction,
};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_tap_presentation::{
    HoldContextMenuTapPresentationModifiers, HoldContextMenuTapSource,
};
use crate::core::widgets::navigations::main_bottom_navigation::MainBottomNavigation;
use crate::core::widgets::navigations::main_tab_bar::MainTabBar;
use crate::core::widgets::navigations::main_tab_bar_settings::{
    RiseTabBarButtonCodable, RiseTabBarPlatform, RiseTabBarSettings, RiseTabBarSettle, RiseTabBarSurface,
};
use crate::core::widgets::wrappers::list_shell_ui::{
    ListShellScrollUnderChromeModifiers, SettingsWrapperUi, page_column_backdrop,
};
use crate::modules::chat::components::chat::backdrop::chat_decorative_backdrop::ChatDecorativeBackdrop;
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_store::ChatWallpaperPreset;
use crate::modules::user::engine::rise_user_tab_bar_presentation::RiseUserTabBarPresentation;
use crate::modules::user::pages::profile::settings::customization_settings::settings::tab_bar_settings::tab_bar_order_group::TabBarOrderGroup;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;

type ToggleApply = fn(&mut RiseTabBarSettings, bool);
type SliderApply = fn(&mut RiseTabBarSettings, i64);
type SettleApply = fn(&mut RiseTabBarSettings, RiseTabBarSettle);

/// page state and the observed stores
struct TabBarSettingsState {
    platform: RiseTabBarPlatform,
    draft: Option<RiseTabBarSettings>,
    settle_menu_row_id: Option<String>,
    preview_tab: MainTab,
    _subscriptions: Vec<Subscription>,
}

impl TabBarSettingsState {
    fn new(cx: &mut Context<Self>) -> Self {
        let theme_store = ThemeStore::shared(cx);
        let presentation = RiseUserTabBarPresentation::shared(cx);
        let transport = SaiWs::manager(cx);
        let browse = TabBarBrowsePreferences::shared(cx);
        Self {
            platform: RiseTabBarPlatform::CURRENT,
            draft: None,
            settle_menu_row_id: None,
            preview_tab: MainTab::Posts,
            _subscriptions: vec![
                cx.observe(&theme_store, |_, _, cx| cx.notify()),
                cx.observe(&presentation, |_, _, cx| cx.notify()),
                cx.observe(&transport, |_, _, cx| cx.notify()),
                cx.observe(&browse, |_, _, cx| cx.notify()),
            ],
        }
    }
}

/// what every row builder needs
struct RowContext {
    state: Entity<TabBarSettingsState>,
    theme: AppTheme,
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

fn commit_draft(state: &Entity<TabBarSettingsState>, cx: &mut App) {
    let (draft, platform) = {
        let state = state.read(cx);
        (state.draft, state.platform)
    };
    let Some(draft) = draft else {
        return;
    };
    UserInteractionsStore::shared(cx).update(cx, |store, cx| store.set_tab_bar_settings(draft, platform, cx));
    state.update(cx, |state, cx| {
        state.draft = None;
        cx.notify();
    });
}

fn surface_title(surface: RiseTabBarSurface, platform: RiseTabBarPlatform) -> String {
    match surface {
        RiseTabBarSurface::Auto => tr("tab_bar_surface_auto"),
        RiseTabBarSurface::Glass => tr("tab_bar_surface_glass"),
        RiseTabBarSurface::Legacy => {
            if platform == RiseTabBarPlatform::Android {
                tr("tab_bar_surface_legacy_android")
            } else {
                tr("tab_bar_surface_legacy")
            }
        },
    }
}

fn settle_title(settle: RiseTabBarSettle) -> String {
    match settle {
        RiseTabBarSettle::Center => tr("tab_bar_settle_center"),
        RiseTabBarSettle::Reveal => tr("tab_bar_settle_reveal"),
        RiseTabBarSettle::ReturnHome => tr("tab_bar_settle_return_home"),
    }
}

/// formats a number with fixed fraction digits in the app locale
fn fixed_fraction(value: f64, digits: usize) -> String {
    format!("{value:.digits$}").replace('.', RiseFormat::decimal_separator())
}

fn percent_label(percent: i64) -> String {
    let fraction = percent as f64 / 100.0;
    PercentNumberFormatter::new(I18n::shared().code())
        .string(fraction)
        .unwrap_or_else(|| RiseFormat::percent(fraction))
}

fn seconds_label(milliseconds: i64) -> String {
    tr("duration_seconds_abbr").replace("%d", &fixed_fraction(milliseconds as f64 / 1_000.0, 2))
}

fn icon_tile(name: &'static str, tile: Hsla) -> impl IntoElement {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(GroupedCardMetrics::ICON_SIDE))
        .bg(tile)
        .clip_shape(Shape::continuous(11.0))
        .children(IconUi::sized(name, px(17.0), white(), false))
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

#[allow(clippy::too_many_arguments)]
fn toggle_row(
    row_context: &RowContext,
    id: &'static str,
    icon: &'static str,
    tile: &'static str,
    title: String,
    subtitle: Option<String>,
    is_on: bool,
    disabled: bool,
    apply: ToggleApply,
) -> GroupedBtn {
    let switch_state = row_context.state.clone();
    let tap_state = row_context.state.clone();
    let mut row = GroupedBtn::new(id, title)
        .icon(icon)
        .icon_background(color(tile))
        .disabled(disabled)
        .show_chevron(false)
        .trailing_slot(SwitchUi::new(
            SharedString::from(format!("{id}-switch")),
            Binding::new(
                move |_| is_on,
                move |value, _, cx| {
                    if !disabled {
                        change(&switch_state, |settings| apply(settings, value), cx);
                    }
                },
            ),
        ))
        .callback(move |_, cx| change(&tap_state, |settings| apply(settings, !is_on), cx));
    if let Some(subtitle) = subtitle {
        row = row.subtitle(subtitle);
    }
    row
}

#[allow(clippy::too_many_arguments)]
fn slider_row(
    row_context: &RowContext,
    id: &'static str,
    icon: &'static str,
    tile: &'static str,
    title: String,
    value: String,
    current: i64,
    range: RangeInclusive<i64>,
    step: i64,
    disabled: bool,
    apply: SliderApply,
) -> GroupedBtn {
    let theme = &row_context.theme;
    let write_state = row_context.state.clone();
    let commit_state = row_context.state.clone();
    let (lower, upper) = (*range.start(), *range.end());
    let body = slider(
        SharedString::from(format!("{id}-slider")),
        Binding::new(
            move |_| current as f64,
            move |raw: f64, _, cx| {
                if disabled {
                    return;
                }
                let stepped = (raw / step as f64).round() as i64 * step;
                let value = stepped.clamp(lower, upper);
                let platform = write_state.read(cx).platform;
                let draft = write_state.read(cx).draft;
                let mut next = match draft {
                    Some(draft) => draft,
                    None => presented_settings(platform, cx),
                };
                apply(&mut next, value);
                write_state.update(cx, |state, cx| {
                    state.draft = Some(next);
                    cx.notify();
                });
            },
        ),
        lower as f64..=upper as f64,
    )
    .step(step as f64)
    .tint(theme.primary._100)
    .on_editing_changed(move |editing, _, cx| {
        if !editing {
            commit_draft(&commit_state, cx);
        }
    });

    GroupedBtn::new(id, title.clone())
        .disabled(disabled)
        .accessibility_value(value.clone())
        .content_slot(
            hstack(GroupedCardMetrics::ICON_SPACING)
                .w_full()
                .items_start()
                .px(px(GroupedCardMetrics::ROW_HORIZONTAL_PADDING))
                .py(px(GroupedCardMetrics::ROW_VERTICAL_PADDING))
                .child(icon_tile(icon, color(tile)))
                .child(
                    vstack(6.0)
                        .flex_1()
                        .min_w(px(0.0))
                        .child(
                            hstack(8.0)
                                .w_full()
                                .min_h(px(GroupedCardMetrics::ICON_SIDE))
                                .child(MainText::new(title).px(15.0).color(theme.text.primary).number_of_lines(2))
                                .child(spacer_min_w(8.0))
                                .child(MainText::new(value).px(14.0).color(theme.primary._100)),
                        )
                        .child(body),
                ),
        )
}

fn settle_row_label(
    icon: &'static str,
    tile: &'static str,
    title: String,
    value: RiseTabBarSettle,
    theme: &AppTheme,
) -> impl IntoElement + use<> {
    hstack(GroupedCardMetrics::ICON_SPACING)
        .w_full()
        .px(px(GroupedCardMetrics::ROW_HORIZONTAL_PADDING))
        .py(px(GroupedCardMetrics::ROW_VERTICAL_PADDING))
        .child(icon_tile(icon, color(tile)))
        .child(MainText::new(title).px(15.0).color(theme.text.primary))
        .child(spacer_min_w(8.0))
        .child(MainText::new(settle_title(value)).px(14.0).color(theme.text.secondary))
        .children(IconUi::sized("chevron.up.chevron.down", px(12.0), theme.text.secondary.opacity(0.75), false))
}

#[allow(clippy::too_many_arguments)]
fn settle_row(
    row_context: &RowContext,
    id: &'static str,
    icon: &'static str,
    tile: &'static str,
    title: String,
    value: RiseTabBarSettle,
    disabled: bool,
    apply: SettleApply,
) -> GroupedBtn {
    let theme = row_context.theme.clone();
    let row = GroupedBtn::new(id, title.clone()).disabled(disabled).accessibility_value(settle_title(value));
    if disabled {
        return row.content_slot(settle_row_label(icon, tile, title, value, &theme));
    }
    let opener = row_context.state.clone();
    let is_menu_presented = Binding::entity(
        row_context.state.clone(),
        move |state: &TabBarSettingsState| state.settle_menu_row_id.as_deref() == Some(id),
        move |state: &mut TabBarSettingsState, is_presented: bool, cx| {
            if !is_presented && state.settle_menu_row_id.as_deref() == Some(id) {
                state.settle_menu_row_id = None;
                cx.notify();
            }
        },
    );
    let actions: HoldContextMenuPresentationActions = Rc::new(move |_: &App| {
        RiseTabBarSettle::ALL
            .into_iter()
            .enumerate()
            .map(|(index, settle)| {
                HoldContextMenuAction::new(index.to_string(), settle_title(settle)).checked(settle == value)
            })
            .collect()
    });
    let picked_state = row_context.state.clone();
    let on_action: HoldContextMenuPresentationOnAction = Rc::new(move |picked: String, _: &mut Window, cx: &mut App| {
        let all = RiseTabBarSettle::ALL;
        let Some(settle) = picked.parse::<usize>().ok().and_then(|index| all.get(index).copied()) else {
            return;
        };
        change(&picked_state, |settings| apply(settings, settle), cx);
    });
    let source_title = title.clone();
    let source_theme = theme.clone();
    let source: HoldContextMenuTapSource = Rc::new(move |_: &mut Window, _: &mut App| {
        settle_row_label(icon, tile, source_title.clone(), value, &source_theme).into_any_element()
    });
    row.content_slot(
        plain(SharedString::from(format!("{id}-button")))
            .on_press(move |_, cx| {
                opener.update(cx, |state, cx| {
                    state.settle_menu_row_id = Some(id.to_owned());
                    cx.notify();
                });
            })
            .child(settle_row_label(icon, tile, title, value, &theme))
            .tap_context_menu(
                SharedString::from(format!("{id}-menu")),
                is_menu_presented,
                HoldContextMenuAlignment::Trailing,
                false,
                false,
                false,
                None,
                actions,
                on_action,
                source,
            ),
    )
}

fn preview_bar(state: &Entity<TabBarSettingsState>, settings: RiseTabBarSettings) -> AnyElement {
    let current_tab = Binding::entity(
        state.clone(),
        |state: &TabBarSettingsState| state.preview_tab,
        |state: &mut TabBarSettingsState, tab, _| state.preview_tab = tab,
    );
    if settings.more_tab {
        MainBottomNavigation::new(current_tab)
            .is_preview(true)
            .order(Some(settings.more_mode_order()))
            .into_any_element()
    } else {
        MainTabBar::new(current_tab, settings).is_preview(true).into_any_element()
    }
}

fn preview(row_context: &RowContext, settings: RiseTabBarSettings, appearance: RiseThemeAppearance) -> impl IntoElement {
    let theme = &row_context.theme;
    let foreground = div().w_full().px(px(12.0)).pb(px(18.0)).child(preview_bar(&row_context.state, settings));

    vstack(8.0)
        .w_full()
        .align(HorizontalAlignment::Leading)
        .child(
            div()
                .px(px(GroupedCardMetrics::TITLE_HORIZONTAL_PADDING))
                .child(MainText::new(tr("tab_bar_preview_title").to_uppercase()).px(12.0).color(theme.text.secondary)),
        )
        .child(
            div().w_full().px(px(GroupedCardMetrics::HORIZONTAL_PADDING)).child(
                div()
                    .relative()
                    .w_full()
                    .h(px(156.0))
                    .child(
                        zstack(Alignment::Bottom)
                            .size_full()
                            .clip_shape(GroupedCardMetrics::SHAPE)
                            .child(
                                ChatDecorativeBackdrop::new()
                                    .ignores_safe_area(false)
                                    .preview_preset(Some(ChatWallpaperPreset::default_preset().clone()))
                                    .animates(false)
                                    .appearance_override(Some(appearance)),
                            )
                            .child(foreground),
                    )
                    .child(centred_stroke(theme.border._200.opacity(0.6), 0.5, GroupedCardMetrics::SHAPE)),
            ),
        )
        .child(
            div()
                .px(px(GroupedCardMetrics::TITLE_HORIZONTAL_PADDING))
                .pt(px(3.0))
                .child(MainText::new(tr("tab_bar_preview_hint")).px(12.0).color(theme.text.secondary)),
        )
}

fn browse_row(row_context: &RowContext, locked: bool, swipe_browses: bool) -> GroupedBtn {
    GroupedBtn::new("tab-bar-swipe-browse", tr("tab_bar_swipe_browse"))
        .icon("hand.draw")
        .icon_background(color("#30B0C7"))
        .subtitle(tr("tab_bar_swipe_browse_hint"))
        .disabled(locked)
        .show_chevron(false)
        .trailing_slot(SwitchUi::new(
            SharedString::from("tab-bar-swipe-browse-switch"),
            Binding::new(
                move |_| swipe_browses,
                move |value, _, cx| {
                    TabBarBrowsePreferences::shared(cx).update(cx, |browse, cx| browse.set_swipe_browses(value, cx));
                },
            ),
        ))
        .callback(move |_, cx| {
            TabBarBrowsePreferences::shared(cx).update(cx, |browse, cx| browse.set_swipe_browses(!swipe_browses, cx));
        })
}

fn surface_group(
    row_context: &RowContext,
    settings: RiseTabBarSettings,
    platform: RiseTabBarPlatform,
    surface_color: Hsla,
) -> impl IntoElement {
    let glass_missing = platform == RiseTabBarPlatform::CURRENT && !RiseTabBarSurface::Glass.draws_glass();
    let items = RiseTabBarSurface::ALL
        .into_iter()
        .map(|surface| {
            let selected = settings.surface == surface;
            let state = row_context.state.clone();
            GroupedBtn::new(format!("tab-bar-surface-{}", surface.raw_value()), surface_title(surface, platform))
                .leading_slot(selection_indicator(selected, &row_context.theme))
                .disabled(settings.more_tab || (surface == RiseTabBarSurface::Glass && glass_missing))
                .accessibility_selected(selected)
                .show_chevron(false)
                .callback(move |_, cx| change(&state, |next| next.surface = surface, cx))
        })
        .collect();
    let mut group = GroupedBtns::new("tab-bar-surface-group", items)
        .title(tr("tab_bar_surface_title").to_uppercase())
        .surface_color(surface_color);
    if glass_missing {
        group = group.end_group_title(tr("tab_bar_surface_glass_unavailable"));
    }
    group
}

fn scrolling_group(
    row_context: &RowContext,
    settings: RiseTabBarSettings,
    platform: RiseTabBarPlatform,
    swipe_browses: bool,
    surface: Hsla,
) -> impl IntoElement {
    let locked = settings.more_tab;
    let mut rows = vec![
        toggle_row(
            row_context,
            "tab-bar-loops",
            "infinity",
            "#5E5CE6",
            tr("tab_bar_loops"),
            Some(tr("tab_bar_loops_hint")),
            settings.loops,
            locked,
            |next, value| next.loops = value,
        ),
        slider_row(
            row_context,
            "tab-bar-threshold",
            "arrow.left.and.right",
            "#0A84FF",
            tr("tab_bar_scroll_threshold"),
            percent_label(settings.scroll_threshold_percent),
            settings.scroll_threshold_percent,
            RiseTabBarSettings::threshold_range(),
            5,
            locked,
            |next, value| next.scroll_threshold_percent = value,
        ),
        slider_row(
            row_context,
            "tab-bar-speed",
            "hare",
            "#FF9F0A",
            tr("tab_bar_speed"),
            format!("\u{00D7}{}", fixed_fraction(settings.speed_tenths as f64 / 10.0, 1)),
            settings.speed_tenths,
            RiseTabBarSettings::speed_range(),
            1,
            locked,
            |next, value| next.speed_tenths = value,
        ),
    ];
    if platform == RiseTabBarPlatform::CURRENT {
        rows.insert(1, browse_row(row_context, locked, swipe_browses));
    }
    GroupedBtns::new("tab-bar-scrolling-group", rows)
        .title(tr("tab_bar_scrolling_title").to_uppercase())
        .surface_color(surface)
        .uses_lazy_rows(false)
}

fn after_pick_group(row_context: &RowContext, settings: RiseTabBarSettings, surface: Hsla) -> impl IntoElement {
    let locked = settings.more_tab;
    GroupedBtns::new(
        "tab-bar-after-pick-group",
        vec![
            settle_row(
                row_context,
                "tab-bar-tap-settle",
                "hand.tap",
                "#00A9BB",
                tr("tab_bar_after_tap"),
                settings.tap_settle,
                locked,
                |next, value| next.tap_settle = value,
            ),
            settle_row(
                row_context,
                "tab-bar-swipe-settle",
                "hand.draw",
                "#00B894",
                tr("tab_bar_after_swipe"),
                settings.swipe_settle,
                locked,
                |next, value| next.swipe_settle = value,
            ),
            slider_row(
                row_context,
                "tab-bar-return-delay",
                "timer",
                "#E84393",
                tr("tab_bar_return_delay"),
                seconds_label(settings.return_delay_ms),
                settings.return_delay_ms,
                RiseTabBarSettings::return_delay_range(),
                50,
                locked || !settings.returns_home(),
                |next, value| next.return_delay_ms = value,
            ),
        ],
    )
    .title(tr("tab_bar_after_pick_title").to_uppercase())
    .end_group_title(tr("tab_bar_after_pick_hint"))
    .surface_color(surface)
    .uses_lazy_rows(false)
}

fn throw_group(row_context: &RowContext, settings: RiseTabBarSettings, surface: Hsla) -> impl IntoElement {
    let locked = settings.more_tab || !settings.throw_available();
    GroupedBtns::new(
        "tab-bar-throw-group",
        vec![
            toggle_row(
                row_context,
                "tab-bar-throw",
                "arrow.uturn.backward",
                "#FF453A",
                tr("tab_bar_throw_back"),
                None,
                settings.throw_back,
                locked,
                |next, value| next.throw_back = value,
            ),
            slider_row(
                row_context,
                "tab-bar-throw-speed",
                "wind",
                "#6C5CE7",
                tr("tab_bar_throw_speed"),
                DecimalNumberFormatter::new(I18n::shared().code()).string(settings.throw_speed.max(0) as u64),
                settings.throw_speed,
                RiseTabBarSettings::throw_speed_range(),
                100,
                locked || !settings.throw_back,
                |next, value| next.throw_speed = value,
            ),
            toggle_row(
                row_context,
                "tab-bar-throw-selects-first",
                "1.circle",
                "#0984E3",
                tr("tab_bar_throw_selects_first"),
                None,
                settings.throw_selects_first,
                locked || !settings.throw_back,
                |next, value| next.throw_selects_first = value,
            ),
        ],
    )
    .title(tr("tab_bar_throw_title").to_uppercase())
    .end_group_title(if settings.more_tab || settings.throw_available() {
        tr("tab_bar_throw_hint")
    } else {
        tr("tab_bar_throw_unavailable")
    })
    .surface_color(surface)
    .uses_lazy_rows(false)
}

fn reset_group(row_context: &RowContext, surface: Hsla, cx: &mut App) -> impl IntoElement + use<> {
    let is_synced = RiseUserTabBarPresentation::shared(cx).read(cx).is_synced();
    let unsent = !is_synced && !RiseTransportStatus::is_connected(cx);
    let state = row_context.state.clone();
    GroupedBtns::new(
        "tab-bar-reset-group",
        vec![
            GroupedBtn::new("tab-bar-reset", tr("tab_bar_reset"))
                .icon("arrow.counterclockwise")
                .icon_background(SettingsIconPalette::reset())
                .show_chevron(false)
                .callback(move |_, cx| {
                    let platform = state.update(cx, |state, cx| {
                        state.draft = None;
                        cx.notify();
                        state.platform
                    });
                    selection_changed();
                    UserInteractionsStore::shared(cx).update(cx, |store, cx| store.reset_tab_bar_settings(platform, cx));
                }),
        ],
    )
    .end_group_title(if unsent { tr("tab_bar_offline_hint") } else { tr("tab_bar_reset_hint") })
    .surface_color(surface)
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
        let (platform, draft) = {
            let state = state.read(cx);
            (state.platform, state.draft)
        };
        let shown = match draft {
            Some(draft) => draft,
            None => presented_settings(platform, cx),
        };
        let swipe_browses = TabBarBrowsePreferences::shared(cx).read(cx).swipe_browses();
        let order_state = state.clone();
        let row_context = RowContext { state, theme: theme.clone() };

        let mode_group = GroupedBtns::new(
            "tab-bar-mode-group",
            vec![toggle_row(
                &row_context,
                "tab-bar-more-tab",
                "ellipsis.circle",
                "#8E8E93",
                tr("tab_bar_more_tab"),
                None,
                shown.more_tab,
                false,
                |next, value| next.more_tab = value,
            )],
        )
        .end_group_title(tr("tab_bar_more_tab_hint"))
        .surface_color(surface);

        let order_group = TabBarOrderGroup::new(shown, surface, move |order, cx| {
            change(
                &order_state,
                |value| {
                    if value.more_tab {
                        *value = value.with_more_mode_order(&order);
                    } else {
                        value.button_order = RiseTabBarButton::normalized(&order);
                    }
                },
                cx,
            );
        });

        let reset = reset_group(&row_context, surface, cx);

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
                        .child(preview(&row_context, shown, appearance))
                        .child(mode_group)
                        .child(order_group)
                        .child(surface_group(&row_context, shown, platform, surface))
                        .child(scrolling_group(&row_context, shown, platform, swipe_browses, surface))
                        .child(after_pick_group(&row_context, shown, surface))
                        .child(throw_group(&row_context, shown, surface))
                        .child(reset)
                        .child(div().flex_none().h(px(80.0))),
                ),
        )
        .on_appear("tab-bar-settings-appear", |_, cx| {
            UserInteractionsStore::shared(cx).update(cx, |store, cx| store.load_tab_bar_settings(false, cx));
        })
    }
}
